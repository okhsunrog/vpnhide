//! `vhhelper core …` end to end against a real core on a temp UDS socket: the APK's only
//! config-mutation path. The helper binary is run as a subprocess exactly as su runs it.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use serde_json::Value;
use vpnhide_core_daemon::{CoreState, Publisher, serve};
use vpnhide_core_rpc::StateSnapshot;

fn helper(socket: &Path, args: &[&str], stdin: Option<&str>) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vhhelper"))
        .arg("core")
        .args(args)
        .env("VPNHIDE_CORE_SOCKET", socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn vhhelper");
    {
        let mut pipe = child.stdin.take().unwrap();
        if let Some(text) = stdin {
            pipe.write_all(text.as_bytes()).unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "vhhelper exit {:?}", out.status);
    serde_json::from_slice(&out.stdout).expect("one JSON reply")
}

struct Core {
    _runtime: tokio::runtime::Runtime,
    _handle: vpnhide_core_daemon::ServerHandle,
    _state: Arc<CoreState>,
}

fn spawn_core(socket: &Path, config: &Path) -> Core {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (state, handle) = runtime.block_on(async {
        let state = CoreState::load(
            Publisher::new(77, StateSnapshot::default()),
            config.to_path_buf(),
            None,
        )
        .expect("load");
        let handle = serve(socket, state.clone()).expect("bind");
        (state, handle)
    });
    Core {
        _runtime: runtime,
        _handle: handle,
        _state: state,
    }
}

const CONFIG: &str = r#"{"version":1,"debug":false,"debugSwitch":false,"apps":{"com.bank":{"java":true,"native":true,"ports":true,"portPolicy":{"mode":"allowlist","rules":[{"protocol":"tcp","start":443,"end":443}]}}},"settings":{"rememberSuperkey":false,"optionalFeatures":[],"autoHideVpnServices":true,"autoHideVpnName":false,"autoHideExcludedPackages":[],"autoHiddenPackages":[]}}"#;

#[test]
fn state_replace_conflict_repair_and_query() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config = dir.path().join("vpnhide_config.json");
    let _core = spawn_core(&socket, &config);

    let state = helper(&socket, &["state"], None);
    assert_eq!(state["status"], "ok");
    assert_eq!(state["state"]["config_status"], "missing");
    assert_eq!(state["state"]["epoch"], 77);
    assert_eq!(state["state"]["config_revision"], 0);

    // Bootstrap Replace from on-disk JSON: core persists it; the helper wrote nothing.
    let reply = helper(&socket, &["replace", "1001", "0"], Some(CONFIG));
    assert_eq!(reply["status"], "accepted", "{reply}");
    assert_eq!(reply["op"], 1001);
    assert_eq!(reply["record"]["settled"], "Confirmed");
    assert_eq!(reply["record"]["phases"][0]["phase"], "Persist");
    assert_eq!(reply["state"]["config_revision"], 1);
    assert_eq!(reply["state"]["config_status"], "readable");
    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let echoed: Value = serde_json::from_str(reply["state"]["config"].as_str().unwrap()).unwrap();
    assert_eq!(on_disk, echoed);
    assert_eq!(
        on_disk["apps"]["com.bank"]["portPolicy"]["rules"][0]["start"],
        443
    );

    // A Replace against a stale base conflicts instead of clobbering.
    let stale = CONFIG.replace("\"debug\":false", "\"debug\":true");
    let reply = helper(&socket, &["replace", "1002", "0"], Some(&stale));
    assert_eq!(reply["status"], "conflict", "{reply}");

    // The same op id with the same payload deduplicates (recovery after a lost reply).
    let reply = helper(&socket, &["replace", "1001", "0"], Some(CONFIG));
    assert_eq!(reply["status"], "accepted", "{reply}");

    // Repair re-activates without a Persist.
    let reply = helper(&socket, &["repair", "1003", "1"], None);
    assert_eq!(reply["status"], "accepted", "{reply}");
    assert_eq!(reply["record"]["settled"], "Confirmed");

    // Receipts are queryable independently; unknown ids say so.
    let reply = helper(&socket, &["operation", "1001"], None);
    assert_eq!(reply["status"], "ok");
    assert_eq!(reply["record"]["op"], 1001);
    let reply = helper(&socket, &["operation", "999"], None);
    assert_eq!(reply["status"], "unknown_op");

    // Malformed input never reaches core as an edit.
    let reply = helper(&socket, &["replace", "1004", "1"], Some("{not json"));
    assert_eq!(reply["status"], "invalid_input");
}

#[test]
fn missing_core_is_unreachable_not_an_error_exit() {
    let dir = tempfile::tempdir().unwrap();
    let reply = helper(&dir.path().join("absent.sock"), &["state"], None);
    assert_eq!(reply["status"], "unreachable");
}
