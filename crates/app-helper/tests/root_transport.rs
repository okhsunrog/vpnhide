use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const SESSION_A: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const SESSION_B: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    boot: String,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "vhhelper-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("config"), "{}").unwrap();
        Self {
            root,
            boot: fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .unwrap()
                .trim()
                .into(),
        }
    }

    fn start(&self, args: &[&str]) -> Child {
        Command::new(env!("CARGO_BIN_EXE_vhhelper"))
            .arg("mutation")
            .arg(self.root.join("lane"))
            .arg(self.root.join("config"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn spawn(&self, args: &[&str], input: &str) -> Child {
        let mut child = self.start(args);
        let framed = if args[0] == "run" {
            format!("vpnhide-script 1 {}\n{input}", input.len())
        } else {
            input.into()
        };
        child
            .stdin
            .take()
            .unwrap()
            .write_all(framed.as_bytes())
            .unwrap();
        child
    }

    fn call(&self, args: &[&str], input: &str) -> Value {
        finish(self.spawn(args, input))
    }

    fn open(&self) {
        let initial = self.call(&["inspect"], "");
        assert_eq!(initial["state"]["revision"], 0);
        assert_eq!(
            self.call(&["open", &self.boot, "0", SESSION_A], "")["status"],
            "ok"
        );
    }

    fn run(&self, sequence: &str, script: &str) -> Value {
        self.call(&["run", &self.boot, SESSION_A, sequence], script)
    }

    fn script(&self, body: &str) -> String {
        format!("cd '{}' || exit 1\n{body}\n", self.root.display())
    }
}

#[test]
fn command_receives_the_supervisors_helper_path() {
    let fixture = Fixture::new();
    fixture.open();
    let script = fixture.script("printf '%s' \"$VPNHIDE_MUTATION_HELPER\" > helper-path");
    assert_eq!(fixture.run("1", &script)["state"]["status"], "finished");
    assert_eq!(
        fs::read_to_string(fixture.root.join("helper-path")).unwrap(),
        env!("CARGO_BIN_EXE_vhhelper")
    );
}

#[test]
fn adopt_opens_in_one_round_trip_and_replaces_a_quiescent_predecessor() {
    let fixture = Fixture::new();
    // A lane never opened in this boot is adopted directly: pre-transport commands
    // are untracked writers, not a reason to demand a reboot.
    let first = fixture.call(&["adopt", SESSION_A], "");
    assert_eq!(first["status"], "ok");
    assert_eq!(first["state"]["session"], SESSION_A);
    assert_eq!(first["config_status"], "readable");
    assert_eq!(fixture.run("1", "exit 0")["state"]["status"], "finished");
    // A quiescent tracked session from a previous app process is replaced atomically.
    let adopted = fixture.call(&["adopt", SESSION_B], "");
    assert_eq!(adopted["status"], "ok");
    assert_eq!(adopted["state"]["session"], SESSION_B);
    assert_eq!(adopted["state"]["sequence"], 0);
    assert_eq!(adopted["state"]["status"], "idle");
    // Repeating a lost reply acknowledges the same session without resetting it.
    assert_eq!(
        fixture.call(&["run", &fixture.boot, SESSION_B, "1"], "exit 0")["state"]["status"],
        "finished"
    );
    let repeated = fixture.call(&["adopt", SESSION_B], "");
    assert_eq!(repeated["status"], "ok");
    assert_eq!(repeated["state"]["sequence"], 1);
    assert_eq!(
        fixture.call(&["adopt", "not-a-uuid"], "")["status"],
        "rejected"
    );
}

#[test]
fn incomplete_stdin_is_not_executed_and_delayed_input_cannot_cross_a_recovery_fence() {
    let fixture = Fixture::new();
    fixture.open();
    let args = ["run", &fixture.boot, SESSION_A, "1"];
    let mut truncated = fixture.start(&args);
    truncated
        .stdin
        .take()
        .unwrap()
        .write_all(b"vpnhide-script 1 100\nexit 0")
        .unwrap();
    assert_eq!(finish(truncated)["status"], "unavailable");
    assert_eq!(fixture.call(&["inspect"], "")["state"]["sequence"], 0);
    let mut delayed = fixture.start(&args);
    assert_eq!(
        fixture.call(&["recover", &fixture.boot, SESSION_A, "1"], "")["state"]["status"],
        "not_started"
    );
    delayed
        .stdin
        .take()
        .unwrap()
        .write_all(b"vpnhide-script 1 6\nexit 0")
        .unwrap();
    assert_eq!(finish(delayed)["status"], "rejected");
}

#[test]
fn config_fifo_is_unavailable_instead_of_blocking_the_lane() {
    let fixture = Fixture::new();
    fixture.open();
    let config = fixture.root.join("config");
    fs::remove_file(&config).unwrap();
    let name = std::ffi::CString::new(config.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert_eq!(
        fixture.call(&["inspect"], "")["config_status"],
        "unavailable"
    );
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn finish(mut child: Child) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("supervisor did not finish within test deadline");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    serde_json::from_slice(&out.stdout).unwrap()
}

fn await_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "missing test handshake: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn finished_receipt_survives_new_client_and_duplicate_run_cannot_write() {
    let fixture = Fixture::new();
    fixture.open();
    let reply = fixture.run("1", &fixture.script("printf changed >config"));
    assert_eq!(reply["state"]["status"], "finished");
    assert_eq!(reply["state"]["exit_code"], 0);
    assert_eq!(reply["config"], "changed");
    assert_eq!(
        fixture.run("1", &fixture.script("printf duplicate >config"))["status"],
        "rejected"
    );
    let readback = fixture.call(&["recover", &fixture.boot, SESSION_A, "1"], "");
    assert_eq!(reply["state"], readback["state"]);
    assert_eq!(
        fs::read_to_string(fixture.root.join("config")).unwrap(),
        "changed"
    );
}

#[test]
fn recovery_fences_a_launch_delayed_before_dispatch() {
    let fixture = Fixture::new();
    fixture.open();
    let recovered = fixture.call(&["recover", &fixture.boot, SESSION_A, "1"], "");
    assert_eq!(recovered["state"]["status"], "not_started");
    assert_eq!(
        fixture.run("1", &fixture.script("printf stale >config"))["status"],
        "rejected"
    );
    assert_eq!(
        fixture.run("2", &fixture.script("printf fresh >config"))["state"]["exit_code"],
        0
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("config")).unwrap(),
        "fresh"
    );
}

#[test]
fn session_cas_rejects_delayed_open_and_commands_from_previous_app_process() {
    let fixture = Fixture::new();
    fixture.open();
    let next = fixture.call(&["open", &fixture.boot, "1", SESSION_B], "");
    assert_eq!(next["status"], "ok");
    assert_eq!(
        fixture.call(&["open", &fixture.boot, "0", SESSION_A], "")["status"],
        "rejected"
    );
    assert_eq!(fixture.run("1", "exit 0")["status"], "rejected");
    // Retrying a lost open response must not reset even a session that already ran a command.
    let completed = fixture.call(&["run", &fixture.boot, SESSION_B, "1"], "exit 0");
    assert_eq!(
        fixture.call(&["open", &fixture.boot, "1", SESSION_B], "")["state"],
        completed["state"]
    );
}

#[test]
fn detached_descendant_holds_lane_after_shell_exit() {
    let fixture = Fixture::new();
    fixture.open();
    let script = fixture.script("(touch entered; i=0; while [ ! -f release ] && [ \"$i\" -lt 500 ]; do sleep 0.01; i=$((i+1)); done; [ -f release ] || exit 124; printf child >config) &\nexit 0");
    let child = fixture.spawn(&["run", &fixture.boot, SESSION_A, "1"], &script);
    await_file(&fixture.root.join("entered"));
    assert_eq!(fixture.call(&["inspect"], "")["status"], "busy");
    assert_eq!(fixture.run("2", "exit 0")["status"], "busy");
    assert_eq!(
        fixture.call(&["recover", &fixture.boot, SESSION_A, "1"], "")["status"],
        "busy"
    );
    fs::write(fixture.root.join("release"), "").unwrap();
    let reply = finish(child);
    assert_eq!(reply["config"], "child");
    assert_eq!(reply["state"]["status"], "finished");
}

#[test]
fn killed_supervisor_never_turns_an_unlocked_lane_into_known_completion() {
    let fixture = Fixture::new();
    fixture.open();
    let script = fixture.script("touch entered; i=0; while [ ! -f release ] && [ \"$i\" -lt 500 ]; do sleep 0.01; i=$((i+1)); done; [ -f release ] || exit 124; printf late >config; touch done");
    let mut child = fixture.spawn(&["run", &fixture.boot, SESSION_A, "1"], &script);
    await_file(&fixture.root.join("entered"));
    child.kill().unwrap();
    child.wait().unwrap();
    let readback = fixture.call(&["recover", &fixture.boot, SESSION_A, "1"], "");
    assert_eq!(readback["state"]["status"], "running");
    assert_eq!(readback["config_status"], "unavailable");
    assert_eq!(fixture.run("2", "exit 0")["status"], "rejected");
    assert_eq!(
        fixture.call(&["open", &fixture.boot, "2", SESSION_B], "")["status"],
        "rejected"
    );
    // One-shot adoption is refused for the same reason: the predecessor is still running.
    assert_eq!(
        fixture.call(&["adopt", SESSION_B], "")["status"],
        "rejected"
    );
    fs::write(fixture.root.join("release"), "").unwrap();
    await_file(&fixture.root.join("done"));
    assert_eq!(fixture.call(&["inspect"], "")["state"]["status"], "running");
}

#[test]
fn failed_command_retains_actual_file_and_redacts_script_and_output() {
    let fixture = Fixture::new();
    fixture.open();
    let reply = fixture.run(
        "1",
        &fixture
            .script("printf partial >config; echo private-secret; echo private-secret >&2; exit 7"),
    );
    assert_eq!(reply["state"]["exit_code"], 7);
    assert_eq!(reply["config"], "partial");
    assert!(!reply.to_string().contains("private-secret"));
    let journal = fs::read_to_string(fixture.root.join("lane/state.json")).unwrap();
    assert!(!journal.contains("private-secret"));
    assert!(!journal.contains("partial"));
}

#[test]
fn unknown_or_missing_journal_is_not_reinitialized() {
    for data in [Some("garbage"), None] {
        let fixture = Fixture::new();
        fixture.open();
        let path = fixture.root.join("lane/state.json");
        match data {
            Some(text) => fs::write(&path, text).unwrap(),
            None => fs::remove_file(&path).unwrap(),
        }
        assert_eq!(fixture.call(&["inspect"], "")["status"], "unavailable");
        assert_eq!(fixture.run("1", "exit 0")["status"], "unavailable");
    }
}

#[test]
fn previous_boot_can_be_replaced_but_cannot_dispatch_in_this_boot() {
    let fixture = Fixture::new();
    fixture.open();
    let path = fixture.root.join("lane/state.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    state["boot"] = json!(SESSION_B);
    state["status"] = json!("running");
    state["sequence"] = json!(1);
    fs::write(&path, state.to_string()).unwrap();
    assert_eq!(
        fixture.call(&["run", SESSION_B, SESSION_A, "2"], "exit 0")["status"],
        "rejected"
    );
    assert_eq!(
        fixture.call(&["open", SESSION_B, "1", SESSION_B], "")["status"],
        "rejected"
    );
    assert_eq!(
        fixture.call(&["open", &fixture.boot, "1", SESSION_B], "")["state"]["status"],
        "idle"
    );
}

#[test]
fn skipped_sequences_and_wrong_boot_are_rejected_without_mutation() {
    let fixture = Fixture::new();
    fixture.open();
    assert_eq!(fixture.run("2", "exit 0")["status"], "rejected");
    assert_eq!(
        fixture.call(&["recover", SESSION_B, SESSION_A, "1"], "")["status"],
        "rejected"
    );
    assert_eq!(fixture.call(&["inspect"], "")["state"]["sequence"], 0);
}

#[test]
fn capacity_warning_survives_recovery_without_retaining_other_output() {
    let fixture = Fixture::new();
    fixture.open();
    let reply = fixture.run("1", "echo private-secret; echo 'vpnhide-warning native_target_cap total=10 cap=8 dropped=2' >&2");
    let expected = json!({"total": 10, "cap": 8, "dropped": 2});
    assert_eq!(reply["state"]["native_capacity"], expected);
    assert_eq!(
        fixture.call(&["recover", &fixture.boot, SESSION_A, "1"], "")["state"]["native_capacity"],
        expected
    );
    let journal = fs::read_to_string(fixture.root.join("lane/state.json")).unwrap();
    assert!(!journal.contains("private-secret"));
    assert!(!reply.to_string().contains("private-secret"));
    let next = fixture.run("2", "echo other-secret");
    assert!(next["state"]["native_capacity"].is_null());
}

#[test]
fn large_output_cannot_block_descendant_drain_or_be_retained_in_receipt() {
    let fixture = Fixture::new();
    fixture.open();
    let reply = fixture.run("1", "head -c 1048576 /dev/zero; echo; echo 'vpnhide-warning native_target_cap total=10 cap=8 dropped=2'");
    assert_eq!(reply["state"]["status"], "finished");
    assert_eq!(reply["state"]["exit_code"], 0);
    assert_eq!(reply["state"]["native_capacity"]["total"], 10);
    assert!(reply.to_string().len() < 1024);
}
