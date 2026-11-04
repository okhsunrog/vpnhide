//! Closed, same-image read-only child protocol. Valid record prefixes survive a
//! late failure; only an exact successful end permits a complete inventory.
use super::{BoundModule, MAX_FILES, MAX_MODULES, Observation};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const SCHEMA: u32 = 1;
const OUTPUT_LIMIT: usize = 128 * 1024;
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Message {
    Module {
        record: BoundModule,
    },
    End {
        schema: u32,
        unbound_files: usize,
        incomplete: bool,
    },
}

pub(super) fn observe() -> Observation {
    // /proc/self/exe retains the executing image even after a module update
    // replaces its pathname. No caller-selected executable, path or shell.
    let mut command = Command::new("/proc/self/exe");
    command
        .args(["probe", "native-standard-modules", "1"])
        .env_clear();
    match vpnhide_bounded_process::capture_command(
        &mut command,
        None,
        Duration::from_secs(3),
        OUTPUT_LIMIT,
    ) {
        Ok(captured) => decode(
            &captured.output.stdout,
            captured.output.status.success() && !captured.timed_out && !captured.truncated,
        ),
        Err(_) => Observation {
            incomplete: true,
            ..Default::default()
        },
    }
}

fn emit(writer: &mut impl Write, message: &Message) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, message)?;
    writer.write_all(b"\n")?;
    writer.flush()
}
pub(crate) fn child() -> i32 {
    let mut stdout = io::stdout().lock();
    let observed = super::observe_at(
        &super::ROOTS.map(PathBuf::from),
        Path::new("/sys/module"),
        0,
        |record| {
            emit(
                &mut stdout,
                &Message::Module {
                    record: record.clone(),
                },
            )
        },
    );
    let end = Message::End {
        schema: SCHEMA,
        unbound_files: observed.unbound_files,
        incomplete: observed.incomplete,
    };
    if emit(&mut stdout, &end).is_ok() {
        0
    } else {
        1
    }
}

fn decode(bytes: &[u8], succeeded: bool) -> Observation {
    let mut observed = Observation {
        incomplete: true,
        ..Default::default()
    };
    if bytes.len() > OUTPUT_LIMIT {
        return observed;
    }
    let mut ended = false;
    let mut invalid = false;
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        if ended {
            invalid = true;
            break;
        }
        match serde_json::from_slice::<Message>(line) {
            Ok(Message::Module { record }) if observed.modules.len() < MAX_MODULES => {
                observed.modules.push(record)
            }
            Ok(Message::End {
                schema: SCHEMA,
                unbound_files,
                incomplete,
            }) if unbound_files <= MAX_FILES => {
                observed.unbound_files = unbound_files;
                observed.incomplete = incomplete;
                ended = true;
            }
            _ => {
                invalid = true;
                break;
            }
        }
    }
    observed.incomplete |= !ended || !succeeded || invalid || bytes.last() != Some(&b'\n');
    observed
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record() -> &'static [u8] {
        br#"{"kind":"Module","record":{"source":"/vendor_dlkm/lib/modules/wifi.ko","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","metadata":{"name":"wifi","vermagic":"5.15.123-android13 SMP","srcversion":null,"build_id":[1,2,3,4]}}}
"#
    }
    fn complete() -> Vec<u8> {
        let mut bytes = record().to_vec();
        bytes.extend_from_slice(
            b"{\"kind\":\"End\",\"schema\":1,\"unbound_files\":3,\"incomplete\":false}\n",
        );
        bytes
    }
    #[test]
    fn independent_frames_need_exact_completion_and_retain_failed_prefix() {
        let bytes = complete();
        let observed = decode(&bytes, true);
        assert!(!observed.incomplete);
        assert_eq!(observed.modules[0].metadata.name, "wifi");
        assert_eq!(observed.unbound_files, 3);
        let timed_out = decode(&bytes, false);
        assert!(timed_out.incomplete);
        assert_eq!(timed_out.modules, observed.modules);
        for suffix in [
            &b""[..],
            &b"{\"kind\":\"End\",\"schema\":"[..],
            &b"garbage\n"[..],
        ] {
            let mut partial = record().to_vec();
            partial.extend_from_slice(suffix);
            let failed = decode(&partial, true);
            assert!(failed.incomplete);
            assert_eq!(failed.modules, observed.modules);
        }
    }
    #[test]
    fn malformed_or_excess_frames_never_claim_completeness() {
        for end in [
            b"{\"kind\":\"End\",\"schema\":2,\"unbound_files\":0,\"incomplete\":false}\n".as_slice(),
            b"{\"kind\":\"End\",\"schema\":1,\"unbound_files\":513,\"incomplete\":false}\n".as_slice(),
            b"{\"kind\":\"End\",\"schema\":1,\"unbound_files\":0,\"incomplete\":false,\"extra\":1}\n".as_slice(),
        ] { assert!(decode(end, true).incomplete); }
        let mut extra = complete();
        extra.extend_from_slice(record());
        assert!(decode(&extra, true).incomplete);
        let mut many = record().repeat(MAX_MODULES + 1);
        many.extend_from_slice(
            b"{\"kind\":\"End\",\"schema\":1,\"unbound_files\":0,\"incomplete\":false}\n",
        );
        let observed = decode(&many, true);
        assert!(observed.incomplete);
        assert_eq!(observed.modules.len(), MAX_MODULES);
        assert!(decode(&vec![b'X'; OUTPUT_LIMIT + 1], true).incomplete);
        assert!(decode(b"", true).incomplete);
    }
}
