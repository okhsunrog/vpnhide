//! Core-owned no-op policy-rule fixture and exact same-boot crash cleanup.
use super::{identity::Environment, invalid, mutate};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
use vpnhide_diagnostic_model::{
    DIAGNOSTIC_RULE_ACTION, DIAGNOSTIC_RULE_PROTOCOL, NativeReadActor, RouteFixtureSpec,
    RouteMeasurement, RouteSurface, RuleRole,
};

const FILE: &str = "diagnostic-rules.json";
const MAX_BYTES: u64 = 4096;
const FAMILIES: [RouteSurface; 2] = [RouteSurface::RuleIpv4, RouteSurface::RuleIpv6];

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    environment: Environment,
    spec: RouteFixtureSpec,
}

struct Store(File);
impl Store {
    fn open(path: &Path) -> io::Result<Self> {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(path)?;
        let meta = directory.metadata()?;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(invalid("rule receipt directory is not private"));
        }
        Ok(Self(directory))
    }

    fn pinned(&self) -> PathBuf {
        format!("/proc/self/fd/{}", self.0.as_raw_fd()).into()
    }

    fn read(&self) -> io::Result<Option<Receipt>> {
        let file = match crate::payloads::open_beneath(&self.0, FILE) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let meta = file.metadata()?;
        if meta.uid() != self.0.metadata()?.uid() || meta.nlink() != 1 || meta.mode() & 0o077 != 0 {
            return Err(invalid("rule receipt ownership is invalid"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid("rule receipt exceeds bound"));
        }
        let receipt: Receipt = serde_json::from_slice(&bytes).map_err(invalid)?;
        if !receipt.spec.valid() || receipt.spec.rule_uid.is_none() || !receipt.environment.valid()
        {
            return Err(invalid("rule receipt identity is invalid"));
        }
        Ok(Some(receipt))
    }

    fn publish(&self, receipt: &Receipt) -> io::Result<()> {
        let mut staged = tempfile::Builder::new()
            .prefix(".diagnostic-rules-")
            .tempfile_in(self.pinned())?;
        staged.write_all(&serde_json::to_vec(receipt).map_err(invalid)?)?;
        staged.as_file().sync_all()?;
        // The receipt is single-owner under the Core lifetime lock. Use a
        // no-clobber publication so uncertainty can never replace prior intent.
        staged
            .persist_noclobber(self.pinned().join(FILE))
            .map_err(|e| e.error)?;
        self.0.sync_all()
    }

    fn clear(&self) -> io::Result<()> {
        fs::remove_file(self.pinned().join(FILE))?;
        self.0.sync_all()
    }

    fn recover(&self) -> io::Result<RouteMeasurement> {
        let Some(receipt) = self.read()? else {
            return Ok(RouteMeasurement {
                observations: Vec::new(),
            });
        };
        let current = Environment::observe()?;
        if !current.same_boot(&receipt.environment) {
            self.clear()?;
            return Ok(RouteMeasurement {
                observations: Vec::new(),
            });
        }
        if current != receipt.environment {
            return Err(invalid("rule receipt belongs to another network namespace"));
        }
        let cleanup = remove(&receipt.spec)?;
        self.clear()?;
        Ok(cleanup)
    }
}

fn request(spec: &RouteFixtureSpec, surface: RouteSurface, role: RuleRole, add: bool) -> Vec<u8> {
    let mut bytes = vec![0; 16];
    bytes[4..6].copy_from_slice(&(if add { 32u16 } else { 33u16 }).to_ne_bytes());
    bytes[6..8].copy_from_slice(&(if add { 0x605u16 } else { 5u16 }).to_ne_bytes());
    bytes[8..12].copy_from_slice(&1u32.to_ne_bytes());
    let family = if surface == RouteSurface::RuleIpv4 {
        libc::AF_INET
    } else {
        libc::AF_INET6
    };
    bytes.extend_from_slice(&[
        family as u8,
        0,
        0,
        0,
        0,
        0,
        0,
        DIAGNOSTIC_RULE_ACTION,
        0,
        0,
        0,
        0,
    ]);
    let mut attr = |kind: u16, value: &[u8]| {
        bytes.extend_from_slice(&((value.len() + 4) as u16).to_ne_bytes());
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(value);
        bytes.resize((bytes.len() + 3) & !3, 0);
    };
    attr(6, &role.priority(spec).to_ne_bytes()); // FRA_PRIORITY
    attr(21, &[DIAGNOSTIC_RULE_PROTOCOL]); // FRA_PROTOCOL
    if let Some(name) = role.output(spec) {
        attr(17, &[name.as_bytes(), &[0]].concat()); // FRA_OIFNAME
    }
    if let Some(uid) = role.uid(spec) {
        let uid = uid.to_ne_bytes();
        attr(20, &[uid, uid].concat()); // FRA_UID_RANGE
        attr(15, &role.table().to_ne_bytes()); // FRA_TABLE
    }
    let length = bytes.len() as u32;
    bytes[..4].copy_from_slice(&length.to_ne_bytes());
    bytes
}

fn complete(sample: &RouteMeasurement) -> io::Result<()> {
    if !sample.bounded()
        || sample.observations.len() != 4
        || sample.observations.iter().any(|v| v.error.is_some())
    {
        return Err(invalid("policy rule dump incomplete"));
    }
    Ok(())
}

fn absent(spec: &RouteFixtureSpec, sample: &RouteMeasurement, check_goto: bool) -> bool {
    sample.observations.iter().all(|v| {
        v.rules.iter().all(|row| {
            RuleRole::ALL.into_iter().all(|role| {
                !role.exposes(spec, row) && (!check_goto || row.goto != Some(role.priority(spec)))
            })
        })
    })
}

fn remove(spec: &RouteFixtureSpec) -> io::Result<RouteMeasurement> {
    let present = vpnhide_checks::rule::observe();
    complete(&present)?;
    for surface in FAMILIES {
        for role in RuleRole::ALL {
            let mut owned = false;
            for view in present
                .observations
                .iter()
                .filter(|v| v.check.surface == surface)
            {
                let mut slot = view.rules.iter().filter(|row| role.exposes(spec, row));
                let first = slot.next();
                if slot.next().is_some() || first.is_some_and(|row| !role.matches(spec, row, true))
                {
                    return Err(invalid(
                        "policy rule identity conflicts with cleanup receipt",
                    ));
                }
                if view.check.actor == NativeReadActor::Libc {
                    owned = first.is_some();
                } else if owned != first.is_some() {
                    return Err(invalid("policy rule cleanup observations disagree"));
                }
            }
            if owned {
                // Independent readback, not the ACK, settles deletion. Include
                // action, preference, protocol and every issued selector.
                let _ = mutate(
                    &request(spec, surface, role, false),
                    "delete diagnostic rule",
                );
            }
        }
    }
    let cleanup = vpnhide_checks::rule::observe();
    complete(&cleanup)?;
    if !absent(spec, &cleanup, false) {
        return Err(invalid("diagnostic policy rule remains after cleanup"));
    }
    Ok(cleanup)
}

pub(super) fn recover(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path.join(FILE))
        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
    {
        return Ok(());
    }
    Store::open(path)?.recover().map(|_| ())
}

pub(super) struct RuleFixture(Store);
impl RuleFixture {
    pub(super) fn create(
        path: &Path,
        spec: &RouteFixtureSpec,
    ) -> io::Result<(Self, RouteMeasurement, RouteMeasurement)> {
        let store = Store::open(path)?;
        let environment = Environment::observe()?;
        let negative = vpnhide_checks::rule::observe();
        complete(&negative)?;
        if !absent(spec, &negative, true) {
            return Err(invalid(
                "diagnostic policy rule slot or goto target is already in use",
            ));
        }
        store.publish(&Receipt {
            environment,
            spec: spec.clone(),
        })?;
        let start = (|| {
            for surface in FAMILIES {
                for role in RuleRole::ALL {
                    mutate(&request(spec, surface, role, true), "add diagnostic rule")?;
                }
            }
            let sample = vpnhide_checks::rule::observe();
            complete(&sample)?;
            if sample.observations.iter().any(|v| {
                !RuleRole::ALL.into_iter().all(|role| {
                    let mut rows = v.rules.iter().filter(|row| role.exposes(spec, row));
                    rows.next()
                        .is_some_and(|row| role.matches(spec, row, false))
                        && rows.next().is_none()
                })
            }) {
                return Err(invalid("diagnostic policy rule readback differs"));
            }
            Ok(sample)
        })();
        match start {
            Ok(start) => Ok((Self(store), negative, start)),
            Err(error) => {
                store.recover()?;
                Err(error)
            }
        }
    }

    pub(super) fn finish(self) -> io::Result<RouteMeasurement> {
        self.0.recover()
    }
}
