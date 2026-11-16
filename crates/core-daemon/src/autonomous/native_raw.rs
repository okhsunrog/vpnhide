//! Root-only backend control for one admitted diagnostic thread.
use crate::boot_time::{BootDeadline, BootInstant};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Seek, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use vpnhide_activator::CoreKpm;
use vpnhide_diagnostic_model::DiagnosticThreadIdentity;
use vpnhide_loader_ipc::image;
use vpnhide_protocol::diagnostic_raw::{self as wire, Command, Phase, Snapshot};
use vpnhide_protocol::hook_ids::Backend;

enum Transport {
    Proc(File),
    Kpm(CoreKpm),
}

impl Transport {
    fn open(backend: u32) -> io::Result<Self> {
        if backend == Backend::Kpm as u32 {
            let lock = crate::backend::kpm_lock_from_env()
                .ok_or_else(|| io::Error::from(io::ErrorKind::Unsupported))?;
            let kpm = CoreKpm::connect(&lock)
                .map_err(other)?
                .ok_or_else(|| io::Error::from(io::ErrorKind::Unsupported))?;
            if !kpm.diagnostic_raw_supported().map_err(other)? {
                return Err(io::Error::from(io::ErrorKind::Unsupported));
            }
            return Ok(Self::Kpm(kpm));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(wire::NODE)?;
        let metadata = file.metadata()?;
        let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
        if !metadata.is_file()
            || metadata.uid() != 0
            || metadata.mode() & 0o077 != 0
            || unsafe { libc::fstatfs(file.as_raw_fd(), &mut fs) } != 0
            || fs.f_type as u64 != libc::PROC_SUPER_MAGIC as u64
        {
            return Err(io::Error::other("RAW endpoint identity unavailable"));
        }
        Ok(Self::Proc(file))
    }

    fn request(&mut self, command: Command) -> io::Result<Snapshot> {
        let bytes = command.encode().map_err(other)?;
        match self {
            Self::Proc(file) => {
                if command != Command::Status && file.write(bytes.as_bytes())? != bytes.len() {
                    return Err(io::Error::other("uncertain short RAW command"));
                }
                file.rewind()?;
                let mut text = String::new();
                let mut reader = BufReader::new(file);
                use std::io::Read;
                let mut prefix = reader.by_ref().take(wire::MAX_REPLY_BYTES as u64 + 1);
                while prefix.read_line(&mut text)? != 0 {
                    if text.ends_with("vpnhide 1 stats\n") {
                        break;
                    }
                }
                if text.len() > wire::MAX_REPLY_BYTES || !text.ends_with("vpnhide 1 stats\n") {
                    return Err(io::Error::other("incomplete RAW control snapshot"));
                }
                let status = vpnhide_protocol::parse_status_fields(text.as_bytes())
                    .and_then(|fields| fields.complete())
                    .ok_or_else(|| other("RAW backend status unavailable"))?;
                let raw = wire::Snapshot::from_proc(&text).map_err(wire_error)?;
                if raw.backend != status.backend {
                    return Err(other("RAW/control backend mismatch"));
                }
                Ok(raw)
            }
            Self::Kpm(kpm) => {
                let reply = kpm.diagnostic_raw(&bytes).map_err(other)?;
                Snapshot::parse(reply.as_bytes()).map_err(wire_error)
            }
        }
    }
}

fn other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

fn wire_error(error: wire::Error) -> io::Error {
    if error == wire::Error::Unsupported {
        io::Error::new(io::ErrorKind::Unsupported, error)
    } else {
        other(error)
    }
}

struct Keepalive {
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

pub struct RawLease {
    transport: Arc<Mutex<Transport>>,
    backend: u32,
    tid: u32,
    uid: u32,
    nonce: u64,
    window: Duration,
    keepalive: Option<Keepalive>,
    released: bool,
}

impl RawLease {
    pub fn arm(backend: u32, tid: u32, uid: u32, nonce: u64) -> io::Result<Self> {
        Self::arm_with(Transport::open(backend)?, backend, tid, uid, nonce)
    }

    #[cfg(feature = "fixture-oracle")]
    pub fn arm_kpm_for_oracle(kpm: CoreKpm, tid: u32, uid: u32, nonce: u64) -> io::Result<Self> {
        Self::arm_with(Transport::Kpm(kpm), Backend::Kpm as u32, tid, uid, nonce)
    }

    fn arm_with(
        mut transport: Transport,
        backend: u32,
        tid: u32,
        uid: u32,
        nonce: u64,
    ) -> io::Result<Self> {
        let capability = transport.request(Command::Status)?;
        if capability.backend != backend || capability.phase != Phase::Released {
            return Err(io::Error::other("RAW endpoint not idle for this backend"));
        }
        let duration_ms = capability.max_ms.min(wire::MAX_LEASE_MS);
        let mut lease = Self {
            transport: Arc::new(Mutex::new(transport)),
            backend,
            tid,
            uid,
            nonce,
            window: Duration::from_millis(u64::from(duration_ms)),
            keepalive: None,
            released: false,
        };
        lease
            .expect(
                Command::Arm {
                    tid,
                    uid,
                    nonce,
                    duration_ms,
                    previous: capability.nonce,
                },
                Phase::Armed,
            )
            .map_err(other)?;
        Ok(lease)
    }

    fn request(&mut self, command: Command) -> io::Result<Snapshot> {
        self.transport.lock().map_err(other)?.request(command)
    }

    fn expect(&mut self, command: Command, phase: Phase) -> io::Result<()> {
        let status = self.request(command)?;
        if !status.matches(self.backend, self.tid, self.uid, self.nonce, phase) {
            return Err(io::Error::other("RAW scope readback mismatch"));
        }
        Ok(())
    }

    pub fn activate(&mut self) -> io::Result<()> {
        self.expect(Command::Status, Phase::Registered)?;
        self.expect(Command::Activate { nonce: self.nonce }, Phase::Active)
    }

    pub(crate) fn maintain(
        &mut self,
        identity: DiagnosticThreadIdentity,
        deadline: BootDeadline,
    ) -> io::Result<()> {
        let (stop, receive) = mpsc::channel();
        let handle = Arc::clone(&self.transport);
        let (backend, nonce) = (self.backend, self.nonce);
        let interval = self.window.mul_f64(0.4).max(Duration::from_millis(1));
        let thread = thread::Builder::new()
            .name("diagnostic-raw".into())
            .spawn(move || {
                let root = Path::new("/proc");
                let tasks = root.join(identity.pid.to_string()).join("task");
                for sequence in 1..=u32::MAX {
                    if receive.recv_timeout(interval) != Err(mpsc::RecvTimeoutError::Timeout)
                        || BootInstant::now().map_or(true, |now| deadline.expired(now))
                        || !image::process_identity(root, identity.pid)
                            .is_ok_and(|p| p.start_ticks == identity.process_start_ticks)
                        || !image::process_identity(&tasks, identity.tid)
                            .is_ok_and(|t| t.start_ticks == identity.thread_start_ticks)
                    {
                        return;
                    }
                    let result = handle.lock().map_err(other).and_then(|mut transport| {
                        transport.request(Command::Renew { nonce, sequence })
                    });
                    if !result.is_ok_and(|state| {
                        state.matches(backend, identity.tid, identity.uid, nonce, Phase::Active)
                            && state.renewal == sequence
                    }) {
                        return;
                    }
                }
            })?;
        self.keepalive = Some(Keepalive { stop, thread });
        Ok(())
    }

    fn stop_keepalive(&mut self) {
        if let Some(Keepalive { stop, thread }) = self.keepalive.take() {
            drop(stop);
            let _ = thread.join();
        }
    }

    #[cfg(feature = "fixture-oracle")]
    pub fn maintain_for_oracle(&mut self, pid: u32) -> io::Result<()> {
        let proc = Path::new("/proc");
        let tasks = proc.join(pid.to_string()).join("task");
        let identity = DiagnosticThreadIdentity {
            uid: self.uid,
            pid,
            process_start_ticks: image::process_identity(proc, pid)?.start_ticks,
            tid: self.tid,
            thread_start_ticks: image::process_identity(&tasks, self.tid)?.start_ticks,
        };
        self.maintain(identity, BootDeadline::from_now(Duration::from_secs(15))?)
    }

    #[cfg(feature = "fixture-oracle")]
    pub fn renew_for_oracle(&mut self, sequence: u32) -> io::Result<Snapshot> {
        self.request(Command::Renew {
            nonce: self.nonce,
            sequence,
        })
    }

    pub fn finish(&mut self) -> io::Result<bool> {
        self.stop_keepalive();
        // A renewal cannot resurrect an expired lease, so this active readback
        // with the original nonce also qualifies the preceding measurement.
        let active = self.expect(Command::Status, Phase::Active).is_ok();
        let status = self.request(Command::Revoke { nonce: self.nonce })?;
        if status.backend != self.backend
            || status.nonce != self.nonce
            || status.phase != Phase::Released
        {
            return Err(io::Error::other("RAW revocation unconfirmed"));
        }
        self.released = true;
        Ok(active)
    }
}

impl Drop for RawLease {
    fn drop(&mut self) {
        self.stop_keepalive();
        if !self.released {
            let _ = self.request(Command::Revoke { nonce: self.nonce });
        }
    }
}
