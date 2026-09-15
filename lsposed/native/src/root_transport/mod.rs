mod execute;
mod input;
mod output;
mod state;

use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use state::{State, Status, Store, VERSION, valid_id};

const MAX_CONFIG: u64 = 2 * 1024 * 1024;

#[derive(Serialize)]
pub struct Reply {
    version: u32,
    status: &'static str,
    boot: String,
    state: Option<State>,
    config_status: &'static str,
    config: Option<String>,
}

impl Reply {
    pub fn error(status: &'static str) -> Self {
        Self {
            version: VERSION,
            status,
            boot: String::new(),
            state: None,
            config_status: "unavailable",
            config: None,
        }
    }
}

/// CLI uses explicit paths so host/device tests can isolate *all* writes from the real canonical file.
pub fn run(args: &[String]) -> io::Result<Reply> {
    #[cfg(target_os = "android")]
    if unsafe { libc::geteuid() } != 0 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    if args.len() < 3 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let directory = Path::new(&args[0]);
    let config = Path::new(&args[1]);
    let verb = args[2].as_str();
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .to_owned();
    if !valid_id(&boot) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    // Read the complete request BEFORE taking the lock. A stalled su stdin cannot occupy the lane.
    let script = if verb == "run" {
        input::read_script()?
    } else {
        Vec::new()
    };
    let mut store = match Store::lock(directory, &boot) {
        Ok(store) => store,
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(Reply::error("busy")),
        Err(e) => return Err(e),
    };
    let accepted = dispatch(&mut store, &boot, verb, &args[3..], &script)?;
    let (config_status, config) = if store.state.quiescent(&boot) {
        read_config(config)
    } else {
        ("unavailable", None)
    };
    Ok(Reply {
        version: VERSION,
        status: if accepted { "ok" } else { "rejected" },
        boot,
        state: Some(store.state.clone()),
        config_status,
        config,
    })
}

fn dispatch(
    store: &mut Store,
    boot: &str,
    verb: &str,
    args: &[String],
    script: &[u8],
) -> io::Result<bool> {
    match (verb, args) {
        ("inspect", []) => Ok(true),
        ("open", [expected_boot, expected_revision, session]) => {
            let revision = number(expected_revision)?;
            if expected_boot != boot || !valid_id(session) {
                return Ok(false);
            }
            // A repeated open acknowledges the same session without resetting its sequence/outcome.
            if store.state.matches(boot, session) {
                return Ok(true);
            }
            if store.state.revision != revision || !store.state.quiescent(boot) {
                return Ok(false);
            }
            let mut state = State::idle(boot);
            state.session = Some(session.clone());
            store.replace(state)?;
            Ok(true)
        }
        ("run" | "recover", [expected_boot, session, sequence]) => {
            let sequence = number(sequence)?;
            if !store.state.matches(expected_boot, session)
                || expected_boot != boot
                || sequence == 0
            {
                return Ok(false);
            }
            if verb == "recover" && store.state.sequence == sequence {
                return Ok(true);
            }
            if !store.state.quiescent(boot) || sequence != store.state.sequence + 1 {
                return Ok(false);
            }
            if verb == "recover" {
                // Fence a command still waiting in su: late launch is rejected, never replayed.
                let mut state = store.state.clone();
                state.sequence = sequence;
                state.status = Status::NotStarted;
                state.exit_code = None;
                state.descendant_failed = false;
                state.native_capacity = None;
                store.replace(state)?;
            } else {
                execute::execute(store, sequence, script, Duration::from_secs(120))?;
            }
            Ok(true)
        }
        _ => Err(io::ErrorKind::InvalidInput.into()),
    }
}

fn number(value: &str) -> io::Result<u64> {
    value
        .parse::<u64>()
        .ok()
        .filter(|n| *n < i64::MAX as u64)
        .ok_or(io::ErrorKind::InvalidInput.into())
}

fn read_config(path: &Path) -> (&'static str, Option<String>) {
    match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => {
            let mut text = String::new();
            if file.metadata().is_ok_and(|meta| meta.is_file())
                && file.take(MAX_CONFIG + 1).read_to_string(&mut text).is_ok()
                && text.len() as u64 <= MAX_CONFIG
            {
                ("readable", Some(text))
            } else {
                ("unavailable", None)
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => ("missing", None),
        Err(_) => ("unavailable", None),
    }
}
