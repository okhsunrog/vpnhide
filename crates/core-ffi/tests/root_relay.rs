//! Diagnostics and root effects end to end against a real core on a temp UDS socket,
//! through the exact functions the Kotlin bindings call.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use vpnhide_core_daemon::diagnostics::ProbeEnv;
use vpnhide_core_daemon::effects::EffectPaths;
use vpnhide_core_daemon::{CoreState, Publisher, serve};
use vpnhide_core_ffi::{collect_diagnostics, logcat_recording, root_effect};
use vpnhide_core_rpc::StateSnapshot;

fn json(reply: String) -> Value {
    serde_json::from_str(&reply).expect("one JSON reply")
}

struct Core {
    _runtime: tokio::runtime::Runtime,
    _handle: vpnhide_core_daemon::ServerHandle,
    state: Arc<CoreState>,
}

fn spawn_core(socket: &Path, config: &Path) -> Core {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (state, handle) = runtime.block_on(async {
        let state = CoreState::load(
            Publisher::new(5, StateSnapshot::default()),
            config.to_path_buf(),
            None,
        )
        .expect("load");
        state.set_probe_env(ProbeEnv {
            core_exe: PathBuf::from("/nonexistent"),
            app_dir: None,
        });
        let handle = serve(socket, state.clone()).expect("bind");
        (state, handle)
    });
    Core {
        _runtime: runtime,
        _handle: handle,
        state,
    }
}

#[test]
fn diagnostics_and_effects_round_trip() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let sock = socket.display().to_string();
    let core = spawn_core(&socket, &dir.path().join("vpnhide_config.json"));

    let request = r#"{"probes":[{"kind":"hook_counters"},{"kind":"ground_truth_checks"}],"clear_dmesg":false}"#;
    let reply = json(rt.block_on(collect_diagnostics(sock.clone(), request.into())));
    assert_eq!(reply["status"], "ok", "{reply}");
    let outputs = reply["report"]["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0]["probe"]["kind"], "hook_counters");
    assert_eq!(reply["report"]["dmesg_cleared"], false);

    // Arbitrary commands do not decode.
    let bad = r#"{"probes":[{"kind":"shell","command":"id"}],"clear_dmesg":false}"#;
    let reply = json(rt.block_on(collect_diagnostics(sock.clone(), bad.into())));
    assert_eq!(reply["status"], "invalid_input");

    // Effects are off until the service enables them.
    let reply = json(rt.block_on(root_effect(
        sock.clone(),
        9,
        r#"{"kind":"clear_superkey"}"#.into(),
    )));
    assert_eq!(reply["status"], "failed", "{reply}");

    let root = dir.path().join("root");
    core.state
        .set_effect_paths(Some(EffectPaths { root: root.clone() }));
    let set = r#"{"kind":"set_superkey","superkey":"secret"}"#;
    let reply = json(rt.block_on(root_effect(sock.clone(), 10, set.into())));
    assert_eq!(reply["status"], "ok", "{reply}");
    assert_eq!(
        std::fs::read_to_string(root.join("data/adb/vpnhide/superkey")).unwrap(),
        "secret\n"
    );
    let empty = r#"{"kind":"set_superkey","superkey":""}"#;
    let reply = json(rt.block_on(root_effect(sock.clone(), 11, empty.into())));
    assert_eq!(reply["status"], "refused");
    let reply = json(rt.block_on(root_effect(sock, 0, r#"{"kind":"full_reset"}"#.into())));
    assert_eq!(reply["status"], "invalid_input");
}

#[test]
fn missing_core_is_unreachable() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("none.sock").display().to_string();
    let request = r#"{"probes":[],"clear_dmesg":false}"#;
    let reply = json(rt.block_on(collect_diagnostics(sock, request.into())));
    assert_eq!(reply["status"], "unreachable");
}

#[test]
fn logcat_recording_round_trip() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("core.sock");
    let sock = socket.display().to_string();
    let core = spawn_core(&socket, &dir.path().join("vpnhide_config.json"));
    let app_dir = dir.path().join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    core.state.set_probe_env(ProbeEnv {
        core_exe: PathBuf::from("/nonexistent"),
        app_dir: Some(app_dir),
    });
    let call = |body: &str| json(rt.block_on(logcat_recording(sock.clone(), body.into())));

    core.state.logcat().set_config(None);
    let reply = call(r#"{"action":"start","session":7}"#);
    assert_eq!(reply["status"], "disabled", "{reply}");

    core.state
        .logcat()
        .set_config(Some(vpnhide_core_daemon::logcat_session::RecorderConfig {
            program: PathBuf::from("sleep"),
            args: vec!["30".into()],
            dir: dir.path().join("core/logcat"),
            max_bytes: 1 << 20,
            max_duration: std::time::Duration::from_secs(60),
            since_start: false,
        }));
    let reply = call(r#"{"action":"status"}"#);
    assert_eq!(reply["status"], "ok", "{reply}");
    assert!(reply["session"].is_null(), "{reply}");

    let reply = call(r#"{"action":"start","session":7}"#);
    assert_eq!(reply["status"], "ok", "{reply}");
    assert_eq!(reply["session"]["state"]["kind"], "recording");
    let reply = call(r#"{"action":"start","session":8}"#);
    assert_eq!(reply["status"], "busy", "{reply}");
    assert_eq!(reply["running"], 7);
    let reply = call(r#"{"action":"stop","session":8}"#);
    assert_eq!(reply["status"], "unknown_session", "{reply}");

    let reply = call(r#"{"action":"stop","session":7}"#);
    assert_eq!(reply["status"], "ok", "{reply}");
    assert_eq!(reply["session"]["state"]["kind"], "stopped");
    let file = reply["session"]["file"].as_str().unwrap().to_owned();
    assert!(std::path::Path::new(&file).exists());
    assert_eq!(call(r#"{"action":"stop","session":7}"#), reply);

    let reply = call(r#"{"action":"record","session":7}"#);
    assert_eq!(reply["status"], "invalid_input", "{reply}");
}
