//! Core-owned loopback listeners and worker-scoped netfilter stimulus.
use super::route_fixture::identity::Environment;
use crate::boot_time::{BootDeadline, BootInstant};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::{TcpListener, UdpSocket},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use vpnhide_activator::{
    DIAGNOSTIC_PORTS_CHAIN as CHAIN, PORTS_CHAIN4, PORTS_CHAIN6, diagnostic_ports_ruleset,
    output_with_input_timeout,
};
use vpnhide_checks::ports_fixture::{endpoint, observe};
use vpnhide_diagnostic_model::{
    DiagnosticError, PortsEvidence, PortsFixtureSpec, PortsVector, PortsWorkerPhase,
    ScopedPortsResult,
};

const RECEIPT: &str = "diagnostic-ports.json";
const TOOL_TIMEOUT: Duration = Duration::from_millis(1500);
const POLL_INTERVAL: Duration = Duration::from_millis(2);
const STREAM_TIMEOUT: Duration = Duration::from_millis(100);

struct Family {
    tools: [&'static str; 2],
    parent: &'static str,
    ipv6: bool,
}

const FAMILIES: [Family; 2] = [
    Family {
        tools: ["iptables", "iptables-restore"],
        parent: PORTS_CHAIN4,
        ipv6: false,
    },
    Family {
        tools: ["ip6tables", "ip6tables-restore"],
        parent: PORTS_CHAIN6,
        ipv6: true,
    },
];

struct ChainState {
    present: bool,
    attached: bool,
    production: bool,
}

impl Family {
    fn command(&self, args: &[&str], script: Option<&str>) -> io::Result<String> {
        let program = self.tools[usize::from(script.is_some())];
        let output = output_with_input_timeout(
            Command::new(program).args(["-w", "1"]).args(args),
            script.map(str::as_bytes),
            TOOL_TIMEOUT,
        )?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(io::Error::other(format!(
                "{program}: {}: {}",
                output.status,
                stderr.trim()
            )));
        }
        String::from_utf8(output.stdout).map_err(io::Error::other)
    }

    fn inspect(&self) -> io::Result<ChainState> {
        let output = self.command(&["-S"], None)?;
        let mut state = ChainState {
            present: false,
            attached: false,
            production: false,
        };
        for line in output.lines() {
            let words: Vec<_> = line.split_whitespace().collect();
            match words.as_slice() {
                ["-N", chain] if *chain == CHAIN => state.present = true,
                ["-A", "OUTPUT", "-j", target] if *target == self.parent => state.production = true,
                ["-A", parent, "-j", target]
                    if *parent == self.parent && *target == CHAIN && !state.attached =>
                {
                    state.attached = true;
                }
                _ if words
                    .windows(2)
                    .any(|pair| matches!(pair, ["-j" | "-g", target] if *target == CHAIN)) =>
                {
                    return Err(io::Error::other(
                        "unexpected reference to diagnostic Ports chain",
                    ));
                }
                _ => {}
            }
        }
        Ok(state)
    }

    fn counters(&self, uid: u32, ports: [u16; 2]) -> io::Result<[u64; 2]> {
        let output = self.command(&["-v", "-S", CHAIN], None)?;
        let uid = uid.to_string();
        let mut counters = [None; 2];
        for line in output.lines() {
            let words: Vec<_> = line.split_whitespace().collect();
            if words.as_slice() == ["-N", CHAIN]
                || words.as_slice() == ["-A", CHAIN, "-j", "RETURN"]
                || words.starts_with(&["-A", CHAIN, "-c"]) && words.ends_with(&["-j", "RETURN"])
            {
                continue;
            }
            let value = |name| {
                words
                    .windows(2)
                    .find_map(|pair| (pair[0] == name).then_some(pair[1]))
            };
            let index = match value("-p") {
                Some("tcp") => 0,
                Some("udp") => 1,
                _ => {
                    return Err(io::Error::other(
                        "unexpected protocol in diagnostic Ports chain",
                    ));
                }
            };
            let reject = match (index, self.ipv6) {
                (0, _) => "tcp-reset",
                (_, false) => "icmp-port-unreachable",
                (_, true) => "icmp6-port-unreachable",
            };
            let destination = if self.ipv6 { "::1/128" } else { "127.0.0.0/8" };
            if !words.starts_with(&["-A", CHAIN])
                || value("-j") != Some("REJECT")
                || value("--reject-with") != Some(reject)
                || value("--uid-owner") != Some(uid.as_str())
                || value("-d") != Some(destination)
                || value("--dport").and_then(|port| port.parse().ok()) != Some(ports[index])
                || counters[index].is_some()
            {
                return Err(io::Error::other(
                    "unexpected rule in diagnostic Ports chain",
                ));
            }
            counters[index] = value("-c").and_then(|count| count.parse::<u64>().ok());
        }
        let [Some(tcp), Some(udp)] = counters else {
            return Err(io::Error::other("missing diagnostic Ports rule counters"));
        };
        Ok([tcp, udp])
    }

    fn restore(&self, script: &str) -> io::Result<()> {
        self.command(&["--noflush"], Some(script)).map(drop)
    }

    fn remove(&self) -> io::Result<()> {
        let observed = self.inspect()?;
        if observed.present {
            let mut script = String::from("*filter\n");
            if observed.attached {
                script.push_str(&format!("-D {} -j {CHAIN}\n", self.parent));
            }
            script.push_str(&format!("-F {CHAIN}\n-X {CHAIN}\nCOMMIT\n"));
            self.restore(&script)?;
        }
        let observed = self.inspect()?;
        if observed.present || observed.attached {
            return Err(io::Error::other("diagnostic Ports chain survived cleanup"));
        }
        Ok(())
    }
}

struct ReceiptDir(File);

impl ReceiptDir {
    fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let meta = file.metadata()?;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(io::Error::other("Ports receipt directory is not private"));
        }
        Ok(Self(file))
    }

    fn pinned(&self) -> PathBuf {
        format!("/proc/self/fd/{}", self.0.as_raw_fd()).into()
    }

    fn publish(&self) -> io::Result<()> {
        let mut file = tempfile::Builder::new()
            .prefix(".diagnostic-ports-")
            .tempfile_in(self.pinned())?;
        serde_json::to_writer(&mut file, &Environment::observe()?).map_err(io::Error::other)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(self.pinned().join(RECEIPT))
            .map_err(|error| error.error)?;
        self.0.sync_all()
    }

    fn recover(&self) -> io::Result<()> {
        let file = match crate::payloads::open_beneath(&self.0, RECEIPT) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let meta = file.metadata()?;
        if meta.uid() != self.0.metadata()?.uid()
            || meta.nlink() != 1
            || meta.mode() & 0o077 != 0
            || meta.len() > 4096
        {
            return Err(io::Error::other("invalid Ports receipt ownership or size"));
        }
        let recorded: Environment =
            serde_json::from_reader(file.take(4096)).map_err(io::Error::other)?;
        if !recorded.valid() {
            return Err(io::Error::other("invalid Ports receipt identity"));
        }
        let current = Environment::observe()?;
        if recorded.same_boot(&current) {
            if recorded != current {
                return Err(io::Error::other(
                    "Ports receipt belongs to another network namespace",
                ));
            }
            // Independently try both families even if one cannot be reclaimed yet.
            let ipv4 = FAMILIES[0].remove();
            let ipv6 = FAMILIES[1].remove();
            ipv4.and(ipv6)?;
        }
        fs::remove_file(self.pinned().join(RECEIPT))?;
        self.0.sync_all()
    }
}

pub(super) fn recover(path: &Path) -> io::Result<()> {
    ReceiptDir::open(path)?.recover()
}

enum Socket {
    Tcp(TcpListener),
    Udp(UdpSocket),
}

impl Socket {
    fn bind(vector: PortsVector) -> io::Result<(Self, u16)> {
        let addr = endpoint(vector, 0);
        match vector {
            PortsVector::Tcp4 | PortsVector::Tcp6 => {
                let socket = TcpListener::bind(addr)?;
                socket.set_nonblocking(true)?;
                let port = socket.local_addr()?.port();
                Ok((Self::Tcp(socket), port))
            }
            PortsVector::Udp4 | PortsVector::Udp6 => {
                let socket = UdpSocket::bind(addr)?;
                socket.set_nonblocking(true)?;
                let port = socket.local_addr()?.port();
                Ok((Self::Udp(socket), port))
            }
        }
    }

    fn echo(&self, challenge: &[u8; 16]) {
        let _ = match self {
            Self::Tcp(listener) => (|| {
                let (mut stream, _) = listener.accept()?;
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(STREAM_TIMEOUT))?;
                stream.set_write_timeout(Some(STREAM_TIMEOUT))?;
                let mut request = [0; 16];
                stream.read_exact(&mut request)?;
                if &request == challenge {
                    stream.write_all(challenge)?;
                }
                Ok::<(), io::Error>(())
            })(),
            Self::Udp(_) => Ok(()),
        };
        if let Self::Udp(socket) = self {
            let mut request = [0; 17];
            if let Ok((len, source)) = socket.recv_from(&mut request)
                && &request[..len] == challenge
            {
                let _ = socket.send_to(challenge, source);
            }
        }
    }
}

struct Echo {
    spec: PortsFixtureSpec,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Echo {
    fn start() -> io::Result<Self> {
        let mut challenge = [0; 16];
        File::open("/dev/urandom")?.read_exact(&mut challenge)?;
        let mut ports = [0; 4];
        let mut sockets = Vec::new();
        for (index, vector) in PortsVector::ALL.into_iter().enumerate() {
            let (socket, port) = Socket::bind(vector)?;
            ports[index] = port;
            sockets.push(socket);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = stop.clone();
        let worker = thread::Builder::new()
            .name("diagnostic-ports-echo".into())
            .spawn(move || {
                while !shutdown.load(Ordering::Relaxed) {
                    for socket in &sockets {
                        socket.echo(&challenge);
                    }
                    thread::sleep(POLL_INTERVAL);
                }
            })?;
        Ok(Self {
            spec: PortsFixtureSpec { ports, challenge },
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for Echo {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    New,
    Before,
    Protected,
    Revoked,
    Closed,
}

pub(super) struct PortsRun {
    stage: Stage,
    deadline: BootDeadline,
    dir: PathBuf,
    uid: u32,
    echo: Option<Echo>,
    receipt: bool,
    baseline: [u64; 4],
    evidence: PortsEvidence,
}

impl PortsRun {
    pub(super) fn new(dir: PathBuf, uid: u32, deadline: BootDeadline) -> Self {
        Self {
            stage: Stage::New,
            deadline,
            dir,
            uid,
            echo: None,
            receipt: false,
            baseline: [0; 4],
            evidence: PortsEvidence::default(),
        }
    }

    pub(super) fn step(
        &mut self,
        phase: PortsWorkerPhase,
        debug: bool,
    ) -> Result<Option<PortsFixtureSpec>, DiagnosticError> {
        if self
            .deadline
            .expired(BootInstant::now().map_err(|_| DiagnosticError::Expired)?)
        {
            return Err(DiagnosticError::Expired);
        }
        let result = match (self.stage, phase) {
            (Stage::New, PortsWorkerPhase::Prepared) => Echo::start().map(|echo| {
                self.echo = Some(echo);
                self.stage = Stage::Before;
            }),
            (Stage::Before, PortsWorkerPhase::Protected { before }) => {
                self.evidence.before = Some(before);
                self.protect().map(|()| self.stage = Stage::Protected)
            }
            (Stage::Protected, PortsWorkerPhase::Revoked { protected }) => {
                self.evidence.protected = Some(protected);
                self.rule_counters()
                    .and_then(|counters| {
                        self.evidence.rules_confirmed = true;
                        self.evidence.rule_matches =
                            std::array::from_fn(|index| counters[index] > self.baseline[index]);
                        if debug {
                            crate::alog::info(&format!(
                                "diagnostics: Ports protected={protected:?} rule_matches={:?}",
                                self.evidence.rule_matches
                            ));
                        }
                        self.revoke()
                    })
                    .map(|()| self.stage = Stage::Revoked)
            }
            (Stage::Revoked, PortsWorkerPhase::Finished { revoked }) => {
                self.evidence.revoked = Some(revoked);
                self.stage = Stage::Closed;
                self.echo = None;
                return Ok(None);
            }
            _ => return Err(DiagnosticError::StaleSession),
        };
        if let Err(error) = result {
            if debug {
                crate::alog::info(&format!("diagnostics: Ports fixture: {error}"));
            }
            self.cleanup(debug);
            return Err(DiagnosticError::Disabled);
        }
        Ok(self.echo.as_ref().map(|echo| echo.spec.clone()))
    }

    fn protect(&mut self) -> io::Result<()> {
        for family in &FAMILIES {
            let state = family.inspect()?;
            if !state.production || state.present || state.attached {
                return Err(io::Error::other(
                    "production Ports jump unavailable or diagnostic chain occupied",
                ));
            }
        }
        let spec = &self.echo.as_ref().expect("Ports endpoints reserved").spec;
        let store = ReceiptDir::open(&self.dir)?;
        store.publish()?;
        self.receipt = true;
        for (index, family) in FAMILIES.iter().enumerate() {
            let [tcp, udp] = [spec.ports[index * 2], spec.ports[index * 2 + 1]];
            family.restore(&diagnostic_ports_ruleset(family.ipv6, self.uid, tcp, udp))?;
        }
        self.evidence.root = Some(observe(spec));
        self.baseline = self.rule_counters()?;
        Ok(())
    }

    fn rule_counters(&self) -> io::Result<[u64; 4]> {
        let spec = &self.echo.as_ref().expect("Ports endpoints reserved").spec;
        let mut counters = [0; 4];
        for (index, family) in FAMILIES.iter().enumerate() {
            let state = family.inspect()?;
            if !state.production || !state.present || !state.attached {
                return Err(io::Error::other("diagnostic Ports jump disappeared"));
            }
            let [tcp, udp] = [spec.ports[index * 2], spec.ports[index * 2 + 1]];
            let observed = family.counters(self.uid, [tcp, udp])?;
            counters[index * 2..index * 2 + 2].copy_from_slice(&observed);
        }
        Ok(counters)
    }

    fn revoke(&mut self) -> io::Result<()> {
        if self.receipt {
            recover(&self.dir)?;
            self.receipt = false;
        }
        Ok(())
    }

    pub(super) fn cleanup(&mut self, debug: bool) -> ScopedPortsResult {
        self.stage = Stage::Closed;
        let clean = match self.revoke() {
            Ok(()) => true,
            Err(error) => {
                if debug {
                    crate::alog::info(&format!("diagnostics: Ports cleanup deferred: {error}"));
                }
                false
            }
        };
        self.echo = None;
        self.evidence.evaluate(clean)
    }
}
