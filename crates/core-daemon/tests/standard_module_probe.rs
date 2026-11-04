//! The real service image must dispatch the closed read-only probe before daemon
//! startup. Host success says nothing about Android vendor-module availability.
use std::{process::Command, time::Duration};

#[test]
fn standard_module_probe_uses_exact_protocol_and_rejects_unsupported_invocations() {
    let root = tempfile::tempdir().unwrap();
    for (args, expected) in [
        (vec!["probe", "native-standard-modules", "1"], 0),
        (vec!["probe", "native-standard-modules", "2"], 2),
        (
            vec!["probe", "native-standard-modules", "1", "/tmp/arbitrary"],
            2,
        ),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_vpnhide-core"));
        command.args(&args).env_clear().current_dir(root.path());
        let captured = vpnhide_bounded_process::capture_command(
            &mut command,
            None,
            Duration::from_secs(5),
            128 * 1024,
        )
        .unwrap();
        assert!(!captured.timed_out && !captured.truncated);
        assert_eq!(captured.output.status.code(), Some(expected));
        if expected == 0 {
            let lines: Vec<_> = captured
                .output
                .stdout
                .split(|b| *b == b'\n')
                .filter(|line| !line.is_empty())
                .map(|line| serde_json::from_slice::<serde_json::Value>(line).unwrap())
                .collect();
            let end = lines.last().expect("exact completion frame");
            assert_eq!(end["kind"], "End");
            assert_eq!(end["schema"], 1);
            assert!(end["incomplete"].is_boolean());
            assert!(
                lines[..lines.len() - 1]
                    .iter()
                    .all(|frame| frame["kind"] == "Module")
            );
        }
        assert!(root.path().read_dir().unwrap().next().is_none());
    }
}
