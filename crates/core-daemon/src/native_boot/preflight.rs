//! Final-byte loader experiment. Production selection does not dispatch it.
//! The isolated QEMU oracle can exercise the same frozen file and durable budget.
#[cfg(any(test, feature = "fixture-oracle"))]
use std::io;
use std::os::unix::fs::FileExt;

use crate::journal::preflight::PreflightPlan;
use crate::payloads::{Catalog, elf, frozen::FrozenPayload};

#[derive(Debug)]
pub struct PreparedPreflight {
    plan: PreflightPlan,
    payload: FrozenPayload,
}

impl PreparedPreflight {
    pub fn from_catalog(
        catalog: &Catalog,
        plan: &PreflightPlan,
        sha256: &str,
    ) -> Result<Self, String> {
        let series = format!("{}.{}", plan.series.major, plan.series.minor);
        let expected = PreflightPlan::from_catalog(catalog, &series, plan.boot_id.clone())?;
        if &expected != plan {
            return Err("preflight plan differs from the complete catalog candidate set".into());
        }
        let source = catalog
            .payloads
            .iter()
            .find(|p| p.metadata.sha256 == sha256)
            .filter(|p| plan.candidates.contains(&p.metadata))
            .ok_or("preflight payload is not a frozen candidate")?;
        if source.metadata.preflight_protocol != Some(1) || source.metadata.init_contract != Some(1)
        {
            return Err("preflight requires both final-file capability contracts".into());
        }
        let payload = FrozenPayload::copy(&source.file).map_err(|e| e.to_string())?;
        if payload.sha256 != source.metadata.sha256 || payload.bytes != source.metadata.bytes {
            return Err("preflight final bytes changed after catalog verification".into());
        }
        let mut bytes = vec![0; payload.bytes as usize];
        payload
            .file()
            .read_exact_at(&mut bytes, 0)
            .map_err(|e| e.to_string())?;
        let capabilities = elf::capabilities(&bytes).map_err(|e| e.to_string())?;
        if capabilities.preflight_protocol != Some(1) || capabilities.init_contract != Some(1) {
            return Err("preflight capability is absent from the immutable final ELF".into());
        }
        elf::inspect_gki(&bytes).map_err(|e| e.to_string())?;
        Ok(Self {
            plan: plan.clone(),
            payload,
        })
    }

    /// Retained for eventual real-load preparation of these exact immutable
    /// bytes. A loader sentinel is not permission to invoke that real load.
    pub fn payload(&self) -> &FrozenPayload {
        &self.payload
    }
    pub fn plan(&self) -> &PreflightPlan {
        &self.plan
    }

    #[cfg(any(test, feature = "fixture-oracle"))]
    fn dispatch_with(
        &self,
        path: &std::path::Path,
        journal: &mut crate::journal::Journal,
        mut controls: impl FnMut() -> io::Result<Controls>,
        syscall: impl FnOnce(&std::fs::File) -> io::Result<()>,
    ) -> Result<crate::journal::preflight::PreflightState, String> {
        use crate::journal::preflight::{PreflightCompletion as Complete, PreflightState as State};
        let before = controls().map_err(|e| format!("preflight controls unavailable: {e}"))?;
        if before.boot_id != self.plan.boot_id {
            return Err("preflight boot changed before dispatch".into());
        }
        crate::journal::preflight::run_preflight_with(
            path,
            journal,
            &self.plan,
            &self.payload.sha256,
            || {
                // Intent sync may have taken time. Refuse a changed environment
                // before entering the kernel; the consumed receipt stays held.
                if controls().as_ref().ok() != Some(&before) {
                    return State::Unknown { errno: None };
                }
                let result = syscall(self.payload.file());
                let errno = result.as_ref().err().and_then(io::Error::raw_os_error);
                let after = controls();
                if after.as_ref().ok() != Some(&before) {
                    return State::Unknown { errno };
                }
                match errno {
                    Some(libc::EUCLEAN) => State::Finished(Complete::CleanSentinel),
                    Some(errno @ libc::ENOEXEC) => {
                        State::Finished(Complete::CandidateRejected { errno })
                    }
                    Some(
                        errno @ (libc::EPERM
                        | libc::EACCES
                        | libc::ENOSYS
                        | libc::ENOKEY
                        | libc::EKEYREJECTED),
                    ) => State::Finished(Complete::LoaderBlocked { errno }),
                    _ => State::Unknown { errno },
                }
            },
        )
    }

    /// This syscall entry exists only in the explicitly enabled fixture oracle,
    /// excluded from production packages. Its external VM deadline is mandatory;
    /// termination leaves the synced Running intent held, never permission to retry.
    #[cfg(feature = "fixture-oracle")]
    pub fn dispatch_oracle(
        &self,
        path: &std::path::Path,
        journal: &mut crate::journal::Journal,
        env: &super::NativeEnv,
    ) -> Result<crate::journal::preflight::PreflightState, String> {
        use std::os::fd::AsRawFd;
        if unsafe { libc::geteuid() } != 0
            || !std::fs::read_to_string("/proc/cmdline")
                .map_err(|e| e.to_string())?
                .split_whitespace()
                .any(|p| p == "vpnhide_fixture_oracle=1")
        {
            return Err(
                "preflight syscall requires the explicitly enabled isolated QEMU boot".into(),
            );
        }
        self.dispatch_with(
            path,
            journal,
            || Controls::observe(env),
            |file| {
                // SAFETY: retained sealed regular file and a fixed, closed parameter
                // string. No force/ignore-vermagic flags or real-load parameters.
                let result = unsafe {
                    libc::syscall(
                        libc::SYS_finit_module,
                        file.as_raw_fd(),
                        c"preflight_only=1".as_ptr(),
                        0 as libc::c_int,
                    )
                };
                if result == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            },
        )
    }
}

#[cfg(any(test, feature = "fixture-oracle"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct Controls {
    boot_id: String,
    kprobes: Vec<u8>,
}

#[cfg(feature = "fixture-oracle")]
impl Controls {
    fn observe(env: &super::NativeEnv) -> io::Result<Self> {
        use std::io::Read;
        let boot_id = super::read_boot_id(&env.boot_id_path);
        if !super::evidence::ownership_clear(env, &boot_id) {
            return Err(io::Error::other(
                "preflight ownership is not positively clear",
            ));
        }
        for path in [
            "/proc/vpnhide_ctl",
            "/proc/vpnhide_diag",
            vpnhide_protocol::diagnostic_raw::NODE,
        ] {
            if !matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound)
            {
                return Err(io::Error::other("preflight endpoint absence unconfirmed"));
            }
        }
        let mut kprobes = Vec::new();
        std::fs::File::open("/sys/kernel/debug/kprobes/list")?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut kprobes)?;
        if kprobes.len() > 1024 * 1024
            || boot_id.is_empty()
            || super::read_boot_id(&env.boot_id_path) != boot_id
        {
            return Err(io::Error::other(
                "preflight controls changed or exceed budget",
            ));
        }
        Ok(Self { boot_id, kprobes })
    }
}

#[cfg(test)]
mod tests;
