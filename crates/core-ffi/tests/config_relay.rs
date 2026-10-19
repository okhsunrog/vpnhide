//! The app's config save path end to end against a real core on a temp UDS socket,
//! through the exact functions the Kotlin bindings call.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use vpnhide_core_daemon::{CoreState, Publisher, serve};
use vpnhide_core_ffi::{config_operation, config_repair, config_replace, config_state};
use vpnhide_core_rpc::StateSnapshot;

fn path(socket: &Path) -> String {
    socket.display().to_string()
}

fn json(reply: String) -> Value {
    serde_json::from_str(&reply).expect("one JSON reply")
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
    let client = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let config = dir.path().join("vpnhide_config.json");
    let _core = spawn_core(&socket, &config);

    let state = json(client.block_on(config_state(path(&socket))));
    assert_eq!(state["status"], "ok");
    assert_eq!(state["state"]["config_status"], "missing");
    assert_eq!(state["state"]["epoch"], 77);
    assert_eq!(state["state"]["config_revision"], 0);

    // Bootstrap Replace from on-disk JSON: core persists it; the app wrote nothing.
    let reply = json(client.block_on(config_replace(path(&socket), 1001, 0, CONFIG.into())));
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
    let reply = json(client.block_on(config_replace(path(&socket), 1002, 0, stale)));
    assert_eq!(reply["status"], "conflict", "{reply}");

    // The same op id with the same payload deduplicates (recovery after a lost reply).
    let reply = json(client.block_on(config_replace(path(&socket), 1001, 0, CONFIG.into())));
    assert_eq!(reply["status"], "accepted", "{reply}");

    // Repair re-activates without a Persist.
    let reply = json(client.block_on(config_repair(path(&socket), 1003, 1)));
    assert_eq!(reply["status"], "accepted", "{reply}");
    assert_eq!(reply["record"]["settled"], "Confirmed");

    // Receipts are queryable independently; unknown ids say so.
    let reply = json(client.block_on(config_operation(path(&socket), 1001)));
    assert_eq!(reply["status"], "ok");
    assert_eq!(reply["record"]["op"], 1001);
    let reply = json(client.block_on(config_operation(path(&socket), 999)));
    assert_eq!(reply["status"], "unknown_op");

    // Malformed input never reaches core as an edit.
    let reply = json(client.block_on(config_replace(path(&socket), 1004, 1, "{not json".into())));
    assert_eq!(reply["status"], "invalid_input");

    // Op id 0 is never valid and is refused before reaching core.
    let reply = json(client.block_on(config_repair(path(&socket), 0, 1)));
    assert_eq!(reply["status"], "invalid_input");
}

#[test]
fn missing_core_is_unreachable_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let client = tokio::runtime::Runtime::new().unwrap();
    let reply = json(client.block_on(config_state(path(&dir.path().join("absent.sock")))));
    assert_eq!(reply["status"], "unreachable");
}
