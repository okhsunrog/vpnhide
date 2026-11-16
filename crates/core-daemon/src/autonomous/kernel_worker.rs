//! A private Core child for kernel diagnostics. No APK/RPC caller can launch it
//! with Core credentials or use its inherited SELinux context.
use super::boot_time::{BootDeadline, BootInstant};
use super::native_exchange::NativeExchange;
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::{fs::MetadataExt, net::UnixStream, process::CommandExt};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use vpnhide_diagnostic_model::{
    DiagnosticThreadIdentity, NativeMeasurement, NativeWorkerInstruction, NativeWorkerPhase,
    ProbeError, RouteFixtureSpec, Sample,
};
use vpnhide_loader_ipc::image;

pub const MODE: &str = "diagnostic-kernel-worker";
const INET_GID: libc::gid_t = 3003;
const FRAME_LIMIT: usize = 512 * 1024;

pub(super) struct KernelPair {
    pub identity: DiagnosticThreadIdentity,
    pub selinux_context: String,
    pub raw: NativeMeasurement,
    pub protected: NativeMeasurement,
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn drop_privileges(uid: u32, gid: u32, parent: libc::pid_t) -> io::Result<()> {
    #[repr(C)]
    struct CapHeader {
        version: u32,
        pid: i32,
    }
    let header = CapHeader {
        version: 0x2008_0522,
        pid: 0,
    };
    let caps = [0u32; 6]; // Effective, permitted and inheritable, two words each.
    // This also runs between fork and exec. Keep it to credential syscalls.
    unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
            || libc::setgroups(1, &INET_GID) != 0
            || libc::setresgid(gid, gid, gid) != 0
            || libc::setresuid(uid, uid, uid) != 0
            || libc::syscall(libc::SYS_capset, &header, caps.as_ptr()) != 0
            || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0
        {
            return Err(io::Error::last_os_error());
        }
        if libc::getppid() != parent {
            return Err(io::Error::from_raw_os_error(libc::ESRCH));
        }
    }
    Ok(())
}

fn identity(pid: u32, uid: u32, gid: u32, context: &str) -> io::Result<DiagnosticThreadIdentity> {
    let proc = Path::new("/proc");
    let birth = image::process_identity(proc, pid)?;
    let process = proc.join(pid.to_string());
    let status = image::bounded_text(&process.join("status"), 16 * 1024)?;
    let field = |key: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .unwrap_or("")
            .trim()
    };
    let matches_ids = |key, id| {
        field(key)
            .split_whitespace()
            .map(|value| value.parse::<u32>().ok())
            .eq([Some(id); 4])
    };
    let net = |p: &Path| std::fs::metadata(p).map(|m| (m.dev(), m.ino()));
    if !matches_ids("Uid:", uid)
        || !matches_ids("Gid:", gid)
        || field("Groups:") != "3003"
        || field("Threads:") != "1"
        || field("NoNewPrivs:") != "1"
        || ["CapInh:", "CapPrm:", "CapEff:", "CapAmb:"]
            .into_iter()
            .any(|key| u64::from_str_radix(field(key), 16) != Ok(0))
        || image::bounded_text(&process.join("attr/current"), 256)?.trim_end_matches(['\0', '\n'])
            != context
        || net(&process.join("ns/net"))? != net(Path::new("/proc/self/ns/net"))?
        || image::process_identity(proc, pid)? != birth
    {
        return Err(io::Error::other(
            "kernel diagnostic child identity mismatch",
        ));
    }
    Ok(DiagnosticThreadIdentity {
        uid,
        pid,
        tid: pid,
        process_start_ticks: birth.start_ticks,
        thread_start_ticks: birth.start_ticks,
    })
}

fn frame(writer: &mut impl Write, value: &impl serde::Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > FRAME_LIMIT {
        return Err(io::Error::other("kernel diagnostic frame exceeds limit"));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

fn receive<T: serde::de::DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut length = [0; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > FRAME_LIMIT {
        return Err(io::Error::other("invalid kernel diagnostic frame length"));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

struct DeadlineReader<'a> {
    stream: &'a UnixStream,
    deadline: BootDeadline,
}
impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let slice = self
                .deadline
                .wait_slice(BootInstant::now()?)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::TimedOut, "kernel diagnostic child deadline")
                })?;
            self.stream.set_read_timeout(Some(slice))?;
            match self.stream.read(buf) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                value => return value,
            }
        }
    }
}

pub(super) fn observe(
    backend: u32,
    uid: u32,
    gid: u32,
    routes: Option<&RouteFixtureSpec>,
) -> io::Result<KernelPair> {
    if uid < 10_000 || gid == 0 || unsafe { libc::geteuid() } != 0 {
        return Err(io::Error::other(
            "kernel diagnostics require Core and an app identity",
        ));
    }
    let context = image::bounded_text(Path::new("/proc/self/attr/current"), 256)?
        .trim_end_matches(['\0', '\n'])
        .to_owned();
    if !context.starts_with("u:r:") {
        return Err(io::Error::other("Core SELinux context unavailable"));
    }
    let deadline = BootDeadline::from_now(Duration::from_millis(u64::from(
        vpnhide_protocol::diagnostic_raw::MAX_TOTAL_MS,
    )))?;
    let parent = unsafe { libc::getpid() };
    let (socket, other) = UnixStream::pair()?;
    socket.set_write_timeout(Some(Duration::from_millis(250)))?;
    let mut command = Command::new("/proc/self/exe");
    command
        .arg(MODE)
        .env_clear()
        .stdin(Stdio::from(OwnedFd::from(other.try_clone()?)))
        .stdout(Stdio::from(OwnedFd::from(other)))
        .stderr(Stdio::null());
    // SAFETY: only async-signal-safe credential and parent-death syscalls.
    unsafe {
        command.pre_exec(move || drop_privileges(uid, gid, parent));
    }
    let child = ChildGuard(command.spawn()?);
    let pid = child.0.id();
    drop(command);
    let mut reader = DeadlineReader {
        stream: &socket,
        deadline,
    };
    if !receive::<bool>(&mut reader)? {
        return Err(io::Error::other("kernel collector did not become ready"));
    }
    let admitted = identity(pid, uid, gid, &context)?;
    let revalidate = || {
        if identity(pid, uid, gid, &context)? != admitted {
            return Err(io::Error::other("kernel diagnostic child identity changed"));
        }
        Ok(())
    };
    let wire = |error| io::Error::other(format!("kernel collector RAW exchange: {error:?}"));
    let (mut exchange, register) = NativeExchange::begin(admitted, Some(backend)).map_err(wire)?;
    if !matches!(register, NativeWorkerInstruction::Register { .. }) {
        return Err(io::Error::other("kernel RAW lease unavailable"));
    }
    frame(&mut &socket, &(routes, register))?;
    let registered: bool = receive(&mut reader)?;
    if !registered {
        return Err(io::Error::other("kernel collector did not register"));
    }
    revalidate()?;
    let raw = exchange
        .step(admitted, NativeWorkerPhase::Registered, routes, deadline)
        .map_err(wire)?;
    frame(&mut &socket, &raw)?;
    let native = Box::new(receive(&mut reader)?);
    revalidate()?;
    let protect = exchange
        .step(
            admitted,
            NativeWorkerPhase::RawMeasured { native },
            routes,
            deadline,
        )
        .map_err(wire)?;
    let raw = exchange
        .receipt()
        .qualified_sample()
        .cloned()
        .ok_or_else(|| io::Error::other("kernel collector RAW sample unqualified"))?;
    frame(&mut &socket, &protect)?;
    let protected: NativeMeasurement = receive(&mut reader)?;
    revalidate()?;
    if !protected.bounded_for_routes(routes) {
        return Err(io::Error::other(
            "kernel collector protected sample malformed",
        ));
    }
    frame(&mut &socket, &true)?;
    Ok(KernelPair {
        identity: admitted,
        selinux_context: context,
        raw,
        protected,
    })
}

pub fn worker() -> io::Result<()> {
    if unsafe { libc::geteuid() } < 10_000
        || unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) } != 1
    {
        return Err(io::Error::other(
            "invalid kernel diagnostic worker credentials",
        ));
    }
    let (mut input, mut output) = (io::stdin().lock(), io::stdout().lock());
    frame(&mut output, &true)?;
    let (routes, register): (Option<RouteFixtureSpec>, NativeWorkerInstruction) =
        receive(&mut input)?;
    let NativeWorkerInstruction::Register { nonce } = register else {
        return Err(io::Error::other("missing kernel RAW registration"));
    };
    if routes.as_ref().is_some_and(|spec| !spec.valid()) {
        return Err(io::Error::other("invalid kernel route fixture"));
    }
    vpnhide_checks::diagnostic_raw::register(nonce)?;
    frame(&mut output, &true)?;
    for expected in [
        NativeWorkerInstruction::ObserveRaw,
        NativeWorkerInstruction::ObserveProtected,
    ] {
        if receive::<NativeWorkerInstruction>(&mut input)? != expected {
            return Err(io::Error::other(
                "out-of-order kernel diagnostic instruction",
            ));
        }
        let mut sample = NativeMeasurement::unavailable(ProbeError::Unsupported);
        sample.links = match vpnhide_checks::fixture::links() {
            Ok(links) => Sample::Value(links),
            Err(error) => Sample::Error(error),
        };
        sample.proc_interfaces = Some(vpnhide_checks::proc_interfaces::observe());
        sample.routes = routes
            .as_ref()
            .map(|spec| vpnhide_checks::route::observe_proc().scoped(spec));
        frame(&mut output, &sample)?;
    }
    let _: bool = receive(&mut input)?;
    Ok(())
}
