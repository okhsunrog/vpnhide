//! RPC admission behavior with a real bounded child, not a synthetic ready state.
use super::*;
use std::{
    fs,
    io::Write,
    os::unix::{
        ffi::OsStrExt,
        fs::{OpenOptionsExt, PermissionsExt},
    },
};
use vpnhide_core_rpc::DiagnosticProbe;

struct ProbeGate {
    _directory: tempfile::TempDir,
    entered: PathBuf,
    release: PathBuf,
}
impl ProbeGate {
    fn new(state: &CoreState) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("probe");
        let entered = directory.path().join("probe.entered");
        let release = directory.path().join("probe.release");
        let shell = {
            let _environment = crate::ports::test_env_lock();
            std::env::split_paths(&std::env::var_os("PATH").expect("host PATH"))
                .map(|p| p.join("sh"))
                .find(|p| {
                    p.metadata()
                        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                })
                .expect("host POSIX shell")
        };
        // Host-only process synchronization. All admission/verdict assertions are Rust.
        fs::write(&executable,format!("#!{}\nprintf x >> \"$0.entered\"\nIFS= read -r gate < \"$0.release\" || exit 65\nprintf '%s\\n' \"$@\"\n",shell.display())).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let path = std::ffi::CString::new(release.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        state.set_probe_env(crate::diagnostics::ProbeEnv {
            core_exe: executable.clone(),
            app_dir: None,
        });
        Self {
            _directory: directory,
            entered,
            release,
        }
    }
    fn executions(&self) -> u64 {
        self.entered.metadata().map_or(0, |m| m.len())
    }
    async fn release(&self) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(mut file) = fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&self.release)
                {
                    file.write_all(b"go\n").unwrap();
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("probe child is reading its gate");
    }
}
impl Drop for ProbeGate {
    fn drop(&mut self) {
        // Also unblock on assertion failure; never leave a test child waiting on IO.
        if let Ok(mut file) = fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&self.release)
        {
            let _ = file.write_all(b"go\n");
        }
    }
}
fn request(packages: bool) -> DiagnosticsRequest {
    DiagnosticsRequest {
        probes: vec![DiagnosticProbe::RootSnapshot { packages }],
        clear_dmesg: false,
    }
}
async fn until(label: &str, ready: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !ready() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect(label);
}
async fn read_live(server: &CoreServer) {
    let published = tokio::time::timeout(
        Duration::from_secs(1),
        server.clone().state_since(Context::current(), 0, 0),
    )
    .await
    .expect("state read stays live");
    assert_eq!(published.epoch, 42);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn competing_root_snapshots_wait_without_busy_or_coalescing_different_requests() {
    let state = test_state();
    let gate = ProbeGate::new(&state);
    let server = CoreServer::new(state.clone());
    let first = tokio::spawn(
        server
            .clone()
            .collect_diagnostics(Context::current(), request(false)),
    );
    until("first root child started", || gate.executions() == 1).await;
    let second = tokio::spawn(
        server
            .clone()
            .collect_diagnostics(Context::current(), request(true)),
    );
    until("one bounded pending root read", || {
        state.root_rpc.pending_permits() == 0
    })
    .await;
    assert_eq!(gate.executions(), 1);
    assert_eq!(state.root_rpc.available_permits(), 0);
    let overflow = tokio::time::timeout(
        Duration::from_secs(1),
        server
            .clone()
            .collect_diagnostics(Context::current(), request(false)),
    )
    .await
    .expect("pending admission is bounded");
    assert_eq!(overflow.outputs[0].exit, -1);
    assert_eq!(overflow.outputs[0].output, "core diagnostics busy");
    read_live(&server).await;
    gate.release().await;
    let first = first.await.unwrap();
    assert_eq!(first.outputs[0].exit, 0);
    assert!(!first.outputs[0].output.contains("--packages"));
    until("queued root child started", || gate.executions() == 2).await;
    gate.release().await;
    let second = second.await.unwrap();
    assert_eq!(second.outputs[0].exit, 0);
    assert!(second.outputs[0].output.contains("--packages"));
    assert_eq!(state.root_rpc.available_permits(), 1);
    assert_eq!(state.root_rpc.pending_permits(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_or_expired_pending_root_reads_do_not_execute_and_fresh_retry_works() {
    let state = test_state();
    let gate = ProbeGate::new(&state);
    let server = CoreServer::new(state.clone());
    let first = tokio::spawn(
        server
            .clone()
            .collect_diagnostics(Context::current(), request(false)),
    );
    until("first root child started", || gate.executions() == 1).await;
    let pending = tokio::spawn(
        server
            .clone()
            .collect_diagnostics(Context::current(), request(false)),
    );
    until("pending root read admitted", || {
        state.root_rpc.pending_permits() == 0
    })
    .await;
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    assert_eq!(state.root_rpc.pending_permits(), 1);
    assert_eq!(state.root_rpc.available_permits(), 0);
    let mut ctx = Context::current();
    ctx.deadline = Instant::now() + Duration::from_millis(10);
    let expired = server
        .clone()
        .collect_diagnostics(ctx, request(false))
        .await;
    assert_eq!(expired.outputs[0].exit, -1);
    assert_eq!(gate.executions(), 1);
    assert_eq!(state.root_rpc.pending_permits(), 1);
    read_live(&server).await;
    gate.release().await;
    assert_eq!(first.await.unwrap().outputs[0].exit, 0);
    state.set_probe_env(crate::diagnostics::ProbeEnv {
        core_exe: "true".into(),
        app_dir: None,
    });
    let fresh = CoreServer::new(state.clone())
        .collect_diagnostics(Context::current(), request(true))
        .await;
    assert_eq!(fresh.outputs[0].exit, 0);
    assert_eq!(state.root_rpc.available_permits(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_running_root_read_keeps_its_permit_until_the_real_child_finishes() {
    let state = test_state();
    let gate = ProbeGate::new(&state);
    let server = CoreServer::new(state.clone());
    let running = tokio::spawn(
        server
            .clone()
            .collect_diagnostics(Context::current(), request(false)),
    );
    until("root child started", || gate.executions() == 1).await;
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    assert_eq!(state.root_rpc.available_permits(), 0);
    read_live(&server).await;
    let mut ctx = Context::current();
    ctx.deadline = Instant::now() + Duration::from_millis(10);
    assert_eq!(
        server
            .clone()
            .collect_diagnostics(ctx, request(false))
            .await
            .outputs[0]
            .exit,
        -1
    );
    assert_eq!(gate.executions(), 1);
    gate.release().await;
    until("real root child released admission", || {
        state.root_rpc.available_permits() == 1
    })
    .await;
    state.set_probe_env(crate::diagnostics::ProbeEnv {
        core_exe: "/nonexistent-core".into(),
        app_dir: None,
    });
    let failed = server
        .collect_diagnostics(Context::current(), request(false))
        .await;
    assert_eq!(failed.outputs[0].exit, -1);
    assert_ne!(failed.outputs[0].output, "core diagnostics busy");
    assert_eq!(state.root_rpc.available_permits(), 1);
}

#[tokio::test]
async fn already_expired_root_request_never_dispatches_even_with_an_idle_slot() {
    let state = test_state();
    let gate = ProbeGate::new(&state);
    let server = CoreServer::new(state.clone());
    let mut ctx = Context::current();
    ctx.deadline = Instant::now() - Duration::from_millis(1);
    let expired = server.collect_diagnostics(ctx, request(false)).await;
    assert_eq!(expired.outputs[0].exit, -1);
    assert_eq!(gate.executions(), 0);
    assert_eq!(state.root_rpc.available_permits(), 1);
    assert_eq!(state.root_rpc.pending_permits(), 1);
}

#[test]
fn cancellation_before_blocking_execution_does_not_start_a_late_root_child() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let state = test_state();
        let gate = ProbeGate::new(&state);
        let server = CoreServer::new(state.clone());
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let occupier = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            let _ = blocked.recv();
        });
        ready.await.unwrap();
        let snapshot = tokio::spawn(
            server
                .clone()
                .collect_diagnostics(Context::current(), request(false)),
        );
        until(
            "snapshot queued behind the occupied blocking thread",
            || state.root_rpc.available_permits() == 0,
        )
        .await;
        assert_eq!(gate.executions(), 0);
        snapshot.abort();
        assert!(snapshot.await.unwrap_err().is_cancelled());
        read_live(&server).await;
        release.send(()).unwrap();
        occupier.await.unwrap();
        until(
            "cancelled queued snapshot released its actual budget",
            || state.root_rpc.available_permits() == 1,
        )
        .await;
        assert_eq!(gate.executions(), 0);
        assert_eq!(state.root_rpc.pending_permits(), 1);
    });
}
