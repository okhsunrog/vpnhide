//! Root-only kernel RAW lease. Android worker authentication and exact-thread
//! birth/namespace checks belong to the coordinator, before arm AND activation.
//! Keeping this descriptor pins the proc endpoint and bounds crash cleanup.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use vpnhide_protocol::diagnostic_raw::{self as wire, Command, Phase, Snapshot};

pub struct RawLease {
    file: File,
    backend: u32,
    tid: u32,
    uid: u32,
    nonce: u64,
}
impl RawLease {
    pub fn arm(backend: u32, tid: u32, uid: u32, nonce: u64) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(Path::new(wire::NODE))?;
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
        let capability = read(&mut file)?;
        if capability.backend != backend || capability.phase != Phase::Released {
            return Err(io::Error::other("RAW endpoint not idle for this backend"));
        }
        let mut lease = Self {
            file,
            backend,
            tid,
            uid,
            nonce,
        };
        lease.command(Command::Arm {
            tid,
            uid,
            nonce,
            duration_ms: capability.max_ms,
        })?;
        lease.expect(Phase::Armed)?;
        Ok(lease)
    }
    /// Isolated fixture oracle: simulate process exit, bypassing Rust Drop's
    /// explicit revoke to test the kernel's final-FD release callback itself.
    #[cfg(feature = "fixture-oracle")]
    pub fn close_without_revoke_for_oracle(self) {
        let mut lease = std::mem::ManuallyDrop::new(self);
        // SAFETY: the owned File is dropped exactly once, without running the
        // RawLease destructor. All other fields are plain integers.
        unsafe {
            std::ptr::drop_in_place(&mut lease.file);
        }
    }

    fn command(&mut self, command: Command) -> io::Result<()> {
        let bytes = command.encode().map_err(io::Error::other)?;
        // One bounded kernel write, never write_all/retry a short uncertain write.
        if self.file.write(bytes.as_bytes())? != bytes.len() {
            return Err(io::Error::other("uncertain short RAW command"));
        }
        Ok(())
    }
    fn expect(&mut self, phase: Phase) -> io::Result<()> {
        let status = read(&mut self.file)?;
        if !status.matches(self.backend, self.tid, self.uid, self.nonce, phase) {
            return Err(io::Error::other("RAW scope readback mismatch"));
        }
        Ok(())
    }
    /// Caller must reauthenticate the original process AND collector thread.
    pub fn activate(&mut self) -> io::Result<()> {
        self.expect(Phase::Registered)?;
        self.command(Command::Activate { nonce: self.nonce })?;
        self.expect(Phase::Active)
    }
    /// An active readback qualifies the completed RAW interval only because
    /// repeated commands cannot renew its fixed deadline. Revoke is independent:
    /// failed RAW evidence must still be cleaned before any protected sample.
    pub fn finish(&mut self) -> io::Result<bool> {
        let active = self.expect(Phase::Active).is_ok();
        self.command(Command::Revoke { nonce: self.nonce })?;
        let status = read(&mut self.file)?;
        if status.backend != self.backend
            || status.nonce != self.nonce
            || status.phase != Phase::Released
        {
            return Err(io::Error::other("RAW revocation unconfirmed"));
        }
        Ok(active)
    }
}
impl Drop for RawLease {
    fn drop(&mut self) {
        // Never clear another nonce. Descriptor release also revokes this owner;
        // contention is bounded by the kernel TTL, not a claim of immediate proof.
        let _ = self.command(Command::Revoke { nonce: self.nonce });
    }
}
fn read(file: &mut File) -> io::Result<Snapshot> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(wire::MAX_REPLY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Snapshot::parse(&bytes).map_err(io::Error::other)
}
