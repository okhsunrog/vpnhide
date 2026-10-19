//! Once-per-boot native backend selection (core-2.0 §4). Core is the activator:
//! backend modules never load themselves, and the installed set only changes
//! across a reboot (new installs sit in `modules_update`).
//!
//! At startup core scans every native candidate: an installed kmod, KPM or Zygisk
//! module (`module.prop`, no `disable`, no `remove`), the always-live built-in
//! kernel (the control node reports 0x4), a live kmod, and a live KPM core did not
//! load (APatch autoload, a standalone `.kpm`). Native backends are mutually
//! exclusive:
//!
//! - more than one candidate: load nothing, deliver no config to anything, retire
//!   Zygisk's targets, publish [`NativeBootOutcome::Conflict`];
//! - exactly one: load it (or adopt it when it is already live);
//! - none: [`NativeBootOutcome::NoBackend`].
//!
//! The decision is persisted with the kernel `boot_id` and stays fixed for the whole
//! boot: a later Save / RepairActivation only delivers config to the selected
//! backend, and a core restart in the same boot re-adopts it. The one exception is a
//! pending KPM authentication, which core re-tries after the superkey is saved.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use vpnhide_activator::{CoreKpm, KmodLoadFailure, KpmRuntimeProbe};
use vpnhide_core_rpc::{
    KmodBootReport, MAX_DMESG_TAIL, NativeBackend, NativeBootOutcome, NativeBootStatus,
    PortsStatus, SelectionMode, ZygiskTargets,
};

use crate::alog;

/// Canonical order for every published backend list.
const ORDER: [NativeBackend; 4] = [
    NativeBackend::Kmod,
    NativeBackend::Builtin,
    NativeBackend::Kpm,
    NativeBackend::Zygisk,
];

pub const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
pub const STATE_FILE_NAME: &str = "native_boot.json";
pub const APATCH_KPMS_DIR: &str = "/data/adb/ap/kpms";
pub const CONFIG_GZ: &str = "/proc/config.gz";
const KMOD_ARTIFACT: &str = "vpnhide_kmod.ko";
const KPM_ARTIFACT: &str = "vpnhide.kpm";

/// Everything the scan reads and writes. Production paths come from [`Self::from_env`];
/// tests point every field at a temp tree.
#[derive(Debug, Clone)]
pub struct NativeEnv {
    /// The kernel control node (`VPNHIDE_KMOD_CTL`).
    pub ctl: PathBuf,
    /// The module root (`VPNHIDE_KERNEL_MODULES`). `None` = no module scan and no loads.
    pub modules: Option<PathBuf>,
    /// KPM management lock (`VPNHIDE_KPM_LOCK`). `None` = KPM is not managed.
    pub kpm_lock: Option<PathBuf>,
    /// The Zygisk module directory (`VPNHIDE_ZYGISK_MODULE`), else `<modules>/vpnhide_zygisk`.
    pub zygisk_module: Option<PathBuf>,
    pub boot_id_path: PathBuf,
    /// Where the boot decision is persisted (core's owner dir).
    pub state_file: PathBuf,
    /// Root under which legacy per-backend state (`/data/adb/vpnhide_*`) is cleaned.
    pub legacy_root: Option<PathBuf>,
    /// APatch's installed-KPM dir: evidence that a VPN Hide KPM may be live.
    pub apatch_kpms: PathBuf,
}

impl NativeEnv {
    /// Production wiring. `None` when the control node is unset (dev sandbox: no native
    /// management at all).
    pub fn from_env(ctl: Option<PathBuf>) -> Option<Self> {
        let ctl = ctl?;
        let modules = crate::backend::kernel_modules_from_env();
        let production = modules.as_deref() == Some(Path::new(crate::backend::MODULES_ROOT));
        Some(Self {
            ctl,
            zygisk_module: crate::zygisk::module_from_env(),
            kpm_lock: crate::backend::kpm_lock_from_env(),
            modules,
            boot_id_path: BOOT_ID_PATH.into(),
            state_file: crate::ownership::owner_dir().join(STATE_FILE_NAME),
            legacy_root: production.then(|| PathBuf::from("/")),
            apatch_kpms: APATCH_KPMS_DIR.into(),
        })
    }

    pub fn module_dir(&self, backend: NativeBackend) -> Option<PathBuf> {
        match backend {
            NativeBackend::Zygisk => self
                .zygisk_module
                .clone()
                .or_else(|| self.modules.as_ref().map(|m| m.join("vpnhide_zygisk"))),
            NativeBackend::Kmod => self.modules.as_ref().map(|m| m.join("vpnhide_kmod")),
            NativeBackend::Kpm => self.modules.as_ref().map(|m| m.join("vpnhide_kpm")),
            NativeBackend::Builtin => None,
        }
    }

    fn staged_dir(&self, backend: NativeBackend) -> Option<PathBuf> {
        let dir = self.module_dir(backend)?;
        let name = dir.file_name()?.to_owned();
        let root = dir.parent()?;
        Some(root.with_file_name("modules_update").join(name))
    }

    fn artifact(&self, backend: NativeBackend) -> Option<PathBuf> {
        let name = match backend {
            NativeBackend::Kmod => KMOD_ARTIFACT,
            NativeBackend::Kpm => KPM_ARTIFACT,
            _ => return None,
        };
        self.module_dir(backend).map(|dir| dir.join(name))
    }
}

/// What the control node says at scan time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelFact {
    Absent,
    Builtin,
    Kmod,
    Foreign(u32),
    Unreadable(String),
}

/// What KernelPatch says at scan time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KpmFact {
    /// KPM management is disabled (no lock path): never loaded, never observed.
    NotManaged,
    /// No KernelPatch runtime on the device.
    NoRuntime,
    /// Authenticated; the VPN Hide KPM is not loaded.
    Absent,
    /// Authenticated; the VPN Hide KPM is loaded.
    Live,
    /// The runtime refused authentication (no valid superkey / `su` grant).
    AuthFailed(String),
    /// The runtime is present but could not be enumerated.
    Broken(String),
}

/// The scan's inputs to [`decide`]; pure data so the selection matrix is unit-tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanFacts {
    pub installed: Vec<NativeBackend>,
    pub staged: Vec<NativeBackend>,
    pub kernel: KernelFact,
    pub kpm: KpmFact,
    /// Independent evidence that a VPN Hide KPM may be live even though KernelPatch
    /// cannot be queried (an APatch-installed `vpnhide` KPM, or core loaded it this boot).
    pub kpm_may_be_live: bool,
}

impl ScanFacts {
    pub fn live(&self) -> Vec<NativeBackend> {
        let mut live = Vec::new();
        match self.kernel {
            KernelFact::Builtin => live.push(NativeBackend::Builtin),
            KernelFact::Kmod => live.push(NativeBackend::Kmod),
            _ => {}
        }
        if self.kpm == KpmFact::Live {
            live.push(NativeBackend::Kpm);
        }
        live
    }
}

/// What the boot should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Nothing to load or configure; publish this outcome.
    Stop(NativeBootOutcome),
    /// The single candidate is already running (or needs no loader).
    Adopt(NativeBackend),
    /// The single candidate must be loaded by core.
    Load(NativeBackend),
}

fn ordered(set: impl IntoIterator<Item = NativeBackend>) -> Vec<NativeBackend> {
    let set: Vec<NativeBackend> = set.into_iter().collect();
    ORDER.into_iter().filter(|b| set.contains(b)).collect()
}

/// The selection policy. Pure.
pub fn decide(facts: &ScanFacts) -> Plan {
    match &facts.kernel {
        KernelFact::Unreadable(detail) => {
            return Plan::Stop(NativeBootOutcome::Unknown {
                detail: format!("kernel control node unreadable: {detail}"),
            });
        }
        KernelFact::Foreign(id) => return Plan::Stop(NativeBootOutcome::ForeignOwner { id: *id }),
        _ => {}
    }
    let live = facts.live();
    let candidates = ordered(facts.installed.iter().chain(live.iter()).copied());
    if candidates.len() > 1 {
        return Plan::Stop(NativeBootOutcome::Conflict {
            installed: ordered(facts.installed.iter().copied()),
            live,
            staged: ordered(facts.staged.iter().copied()),
        });
    }
    let unverifiable = match &facts.kpm {
        KpmFact::AuthFailed(detail) => Some((true, detail.clone())),
        KpmFact::Broken(detail) => Some((false, detail.clone())),
        _ => None,
    };
    if let Some((auth, detail)) = unverifiable {
        // Nothing else to select: the only backend that could exist is a KPM core cannot
        // see yet (e.g. embedded in boot.img), so report why instead of NoBackend.
        if candidates.contains(&NativeBackend::Kpm) || candidates.is_empty() {
            return Plan::Stop(if auth {
                NativeBootOutcome::AwaitingAuthentication { detail }
            } else {
                NativeBootOutcome::KpmUnverifiable { detail }
            });
        }
        if facts.kpm_may_be_live {
            return Plan::Stop(NativeBootOutcome::KpmUnverifiable { detail });
        }
        // No KPM candidate and no sign of a VPN Hide KPM: an unauthenticated
        // KernelPatch must not block a kmod/built-in/Zygisk setup.
    }
    match candidates.first() {
        None => Plan::Stop(NativeBootOutcome::NoBackend),
        Some(&backend) => match backend {
            NativeBackend::Builtin | NativeBackend::Zygisk => Plan::Adopt(backend),
            _ if live.contains(&backend) => Plan::Adopt(backend),
            _ => Plan::Load(backend),
        },
    }
}

/// The backend core selected this boot, with what it knows of its load arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Selection {
    pub backend: NativeBackend,
    pub mode: SelectionMode,
    pub filesystem_hiding: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Persisted {
    status: NativeBootStatus,
    selection: Option<Selection>,
}

/// The side effects of a boot scan, injectable for tests.
pub trait Effects {
    fn insmod(&self, ctl: &Path, artifact: &Path, fs: bool) -> Result<(), KmodLoadFailure>;
    fn kpm_load(&self, kpm: &CoreKpm, ctl: &Path, artifact: &Path, fs: bool) -> Result<(), String>;
    /// `(uname -r, the KPM supports it)`.
    fn kpm_kernel(&self) -> Result<(String, bool), String>;
    fn kprobes(&self) -> (Option<bool>, Option<bool>);
    fn dmesg_tail(&self) -> Vec<String>;
    fn uname(&self) -> String;
}

/// The device.
pub struct SystemEffects;

impl Effects for SystemEffects {
    fn insmod(&self, ctl: &Path, artifact: &Path, fs: bool) -> Result<(), KmodLoadFailure> {
        vpnhide_activator::insmod_selected_kmod(ctl, artifact, fs)
    }

    fn kpm_load(&self, kpm: &CoreKpm, ctl: &Path, artifact: &Path, fs: bool) -> Result<(), String> {
        kpm.load_selected(ctl, artifact, fs)
            .map_err(|e| e.to_string())
    }

    fn kpm_kernel(&self) -> Result<(String, bool), String> {
        vpnhide_activator::running_kernel_kpm_support().map_err(|e| e.to_string())
    }

    fn kprobes(&self) -> (Option<bool>, Option<bool>) {
        kprobe_config(read_gzip_text(Path::new(CONFIG_GZ)).as_deref())
    }

    fn dmesg_tail(&self) -> Vec<String> {
        dmesg_tail(&vpnhide_activator::sys::kernel_log().unwrap_or_default())
    }

    fn uname(&self) -> String {
        vpnhide_activator::sys::uname()
            .map(|u| u.release)
            .unwrap_or_default()
    }
}

/// A gzip file's text (`zcat`), decoded in-process.
pub fn read_gzip_text(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(file.take(64 << 20))
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// `CONFIG_KPROBES` / `CONFIG_KRETPROBES` from a kernel config. An option absent from a
/// readable config is off; an unreadable config is unknown.
pub fn kprobe_config(config: Option<&str>) -> (Option<bool>, Option<bool>) {
    let option = |name: &str| {
        config.map(|raw| {
            raw.lines().any(|line| {
                line.strip_prefix(name)
                    .and_then(|rest| rest.strip_prefix('='))
                    .is_some_and(|v| v.trim() == "y")
            })
        })
    };
    (option("CONFIG_KPROBES"), option("CONFIG_KRETPROBES"))
}

/// The kernel-log lines that explain a kmod load, newest [`MAX_DMESG_TAIL`] kept.
pub fn dmesg_tail(raw: &str) -> Vec<String> {
    let mut lines: Vec<String> = raw
        .lines()
        .filter(|line| {
            let line = line.to_ascii_lowercase();
            ["vpnhide", "kretprobe", "modules.verify", "version magic"]
                .iter()
                .any(|needle| line.contains(needle))
        })
        .map(str::to_owned)
        .collect();
    if lines.len() > MAX_DMESG_TAIL {
        lines.drain(..lines.len() - MAX_DMESG_TAIL);
    }
    lines
}

/// insmod's EKEYREJECTED, as exit code or strerror text.
pub fn signature_rejected(exit: i32, stderr: &str, dmesg: &[String]) -> bool {
    let text = stderr.to_ascii_lowercase();
    exit == 129
        || text.contains("key was rejected")
        || text.contains("required key not available")
        || dmesg.iter().any(|l| {
            l.to_ascii_lowercase()
                .contains("module verification failed")
        })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn eligible(dir: &Path) -> bool {
    dir.join("module.prop").is_file()
        && !dir.join("disable").exists()
        && !dir.join("remove").exists()
}

fn read_boot_id(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

fn kernel_fact(ctl: &Path) -> KernelFact {
    match vpnhide_activator::observed_kernel_backend(ctl) {
        Ok(None) => KernelFact::Absent,
        Ok(Some(id)) => match crate::backend::kernel_backend_from_id(id) {
            Some(NativeBackend::Builtin) => KernelFact::Builtin,
            Some(_) => KernelFact::Kmod,
            None => KernelFact::Foreign(id),
        },
        Err(e) => KernelFact::Unreadable(e.to_string()),
    }
}

/// Probe KernelPatch through core's management channel.
pub fn probe_kpm(lock: Option<&Path>) -> (KpmFact, Option<CoreKpm>) {
    let Some(lock) = lock else {
        // Unmanaged KPM is never loaded or configured, but a live one still counts.
        return match vpnhide_activator::observed_kpm_loaded() {
            Ok(true) => (KpmFact::Live, None),
            Ok(false) => (KpmFact::NotManaged, None),
            Err(e) => (KpmFact::Broken(e.to_string()), None),
        };
    };
    match CoreKpm::probe(lock) {
        Ok(KpmRuntimeProbe::NoRuntime) => (KpmFact::NoRuntime, None),
        Ok(KpmRuntimeProbe::AwaitingAuthentication(detail)) => (KpmFact::AuthFailed(detail), None),
        Ok(KpmRuntimeProbe::Ready(client)) => match client.status() {
            Ok(Some(_)) => (KpmFact::Live, Some(client)),
            Ok(None) => (KpmFact::Absent, Some(client)),
            Err(e) => (KpmFact::Broken(e.to_string()), None),
        },
        Err(e) => (KpmFact::Broken(e.to_string()), None),
    }
}

/// True when an APatch-installed KPM names itself `vpnhide` (it would be autoloaded at
/// boot without core, and cannot be seen while KernelPatch refuses authentication).
pub fn apatch_installed_vpnhide_kpm(dir: &Path) -> bool {
    fn scan(dir: &Path, depth: u32) -> bool {
        let Ok(entries) = fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() && depth > 0 => scan(&path, depth - 1),
                Ok(t) if t.is_file() => {
                    let mut bytes = Vec::new();
                    fs::File::open(&path)
                        .and_then(|f| f.take(8 << 20).read_to_end(&mut bytes))
                        .is_ok()
                        && bytes.windows(12).any(|w| w == b"name=vpnhide")
                }
                _ => false,
            }
        })
    }
    scan(dir, 2)
}

/// Gather the scan facts. `kpm` is the already-probed KernelPatch fact.
pub fn gather(env: &NativeEnv, kpm: KpmFact) -> ScanFacts {
    let mut installed = Vec::new();
    let mut staged = Vec::new();
    for backend in [
        NativeBackend::Kmod,
        NativeBackend::Kpm,
        NativeBackend::Zygisk,
    ] {
        let Some(dir) = env.module_dir(backend) else {
            continue;
        };
        if eligible(&dir) {
            installed.push(backend);
        } else if !dir.exists()
            && env
                .staged_dir(backend)
                .is_some_and(|s| s.join("module.prop").is_file())
        {
            staged.push(backend);
        }
    }
    ScanFacts {
        installed,
        staged,
        kernel: kernel_fact(&env.ctl),
        kpm,
        kpm_may_be_live: apatch_installed_vpnhide_kpm(&env.apatch_kpms),
    }
}

fn read_prop(dir: &Path, key: &str) -> String {
    fs::read_to_string(dir.join("module.prop"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('=').map(str::to_owned))
        .unwrap_or_default()
}

/// Core's per-boot native state: the published status plus the fixed selection.
#[derive(Debug, Clone)]
pub struct NativeBoot {
    pub env: NativeEnv,
    pub status: NativeBootStatus,
    pub selection: Option<Selection>,
}

impl NativeBoot {
    /// The selected backend for config delivery, if this boot has one.
    pub fn selected(&self) -> Option<NativeBackend> {
        self.selection.map(|s| s.backend)
    }

    /// Awaiting a KernelPatch superkey: the one decision core re-tries within a boot.
    pub fn pending_authentication(&self) -> bool {
        matches!(
            self.status.outcome,
            NativeBootOutcome::AwaitingAuthentication { .. }
                | NativeBootOutcome::KpmUnverifiable { .. }
        )
    }

    /// Start-of-run decision: reuse this boot's persisted decision (a core restart),
    /// otherwise scan, load and retire.
    pub fn start(env: NativeEnv, epoch: u64, filesystem_hiding: bool) -> Self {
        let boot_id = read_boot_id(&env.boot_id_path);
        if let Some(prior) = read_persisted(&env.state_file)
            .filter(|p| !boot_id.is_empty() && p.status.boot_id == boot_id)
        {
            let boot = NativeBoot {
                env: env.clone(),
                status: prior.status,
                selection: prior.selection,
            };
            if !boot.pending_authentication() {
                alog::info(&format!(
                    "native: re-adopting this boot's decision {:?}",
                    boot.status.outcome
                ));
                return boot;
            }
        }
        let (kpm, client) = probe_kpm(env.kpm_lock.as_deref());
        let facts = gather(&env, kpm);
        let boot = Self::decide_and_apply(
            env,
            epoch,
            &boot_id,
            &facts,
            client,
            filesystem_hiding,
            &SystemEffects,
        );
        boot.cleanup_legacy(&facts);
        boot.persist();
        boot
    }

    /// Re-decide after a superkey change, only while authentication is pending.
    /// Returns true when the decision changed.
    pub fn retry_authentication(&mut self, epoch: u64, filesystem_hiding: bool) -> bool {
        if !self.pending_authentication() {
            return false;
        }
        let (kpm, client) = probe_kpm(self.env.kpm_lock.as_deref());
        let facts = gather(&self.env, kpm);
        let next = Self::decide_and_apply(
            self.env.clone(),
            epoch,
            &self.status.boot_id.clone(),
            &facts,
            client,
            filesystem_hiding,
            &SystemEffects,
        );
        let changed = next.status.outcome != self.status.outcome;
        let ports = self.status.ports.clone();
        *self = next;
        self.status.ports = ports;
        self.persist();
        changed
    }

    /// Decide and perform the load / retire for `facts`.
    pub fn decide_and_apply(
        env: NativeEnv,
        epoch: u64,
        boot_id: &str,
        facts: &ScanFacts,
        kpm: Option<CoreKpm>,
        filesystem_hiding: bool,
        effects: &dyn Effects,
    ) -> Self {
        let plan = decide(facts);
        let mut kmod_report = None;
        let (outcome, selection) = match plan {
            Plan::Stop(outcome) => (outcome, None),
            Plan::Adopt(backend) => {
                let selection = Selection {
                    backend,
                    mode: SelectionMode::Adopted,
                    filesystem_hiding: None,
                };
                (selected_outcome(selection), Some(selection))
            }
            Plan::Load(NativeBackend::Kmod) => {
                let (outcome, report) = load_kmod(&env, filesystem_hiding, effects);
                kmod_report = Some(report);
                outcome
            }
            Plan::Load(NativeBackend::Kpm) => {
                load_kpm(&env, kpm.as_ref(), filesystem_hiding, effects)
            }
            Plan::Load(other) => (
                NativeBootOutcome::Unknown {
                    detail: format!("{other:?} has no loader"),
                },
                None,
            ),
        };
        let now = now_ms();
        let mut boot = NativeBoot {
            status: NativeBootStatus {
                boot_id: boot_id.to_owned(),
                decided_epoch: epoch,
                decided_at_ms: now,
                updated_at_ms: now,
                outcome,
                installed: ordered(facts.installed.iter().copied()),
                live: facts.live(),
                staged: ordered(facts.staged.iter().copied()),
                kmod: kmod_report,
                zygisk_targets: ZygiskTargets::NotInstalled,
                ports: None,
            },
            selection,
            env,
        };
        boot.status.zygisk_targets = boot.settle_zygisk(facts);
        alog::info(&format!(
            "native: boot decision {:?} (installed={:?} live={:?} staged={:?} zygisk={:?})",
            boot.status.outcome,
            boot.status.installed,
            boot.status.live,
            boot.status.staged,
            boot.status.zygisk_targets
        ));
        boot
    }

    /// Zygisk keeps hooking whatever `targets.txt` lists. When it is not this boot's
    /// backend, replace that with an empty snapshot so it cannot act as a hidden second
    /// backend.
    fn settle_zygisk(&self, facts: &ScanFacts) -> ZygiskTargets {
        if !facts.installed.contains(&NativeBackend::Zygisk) {
            return ZygiskTargets::NotInstalled;
        }
        if self.selected() == Some(NativeBackend::Zygisk) {
            return ZygiskTargets::Pending;
        }
        let Some(dir) = self.env.module_dir(NativeBackend::Zygisk) else {
            return ZygiskTargets::NotInstalled;
        };
        match retire_zygisk(&dir) {
            Ok(()) => ZygiskTargets::Retired,
            Err(detail) => {
                alog::info(&format!("native: retiring Zygisk targets failed: {detail}"));
                ZygiskTargets::RetireFailed { detail }
            }
        }
    }

    /// Record a config delivery to the selected backend.
    pub fn record_delivery(&mut self, result: Result<(), String>) {
        let Some(selection) = self.selection else {
            return;
        };
        self.status.outcome = match result {
            Ok(()) => {
                if selection.backend == NativeBackend::Zygisk {
                    self.status.zygisk_targets = ZygiskTargets::Published;
                }
                selected_outcome(selection)
            }
            Err(detail) => {
                alog::info(&format!(
                    "native: config delivery to {:?} failed: {detail}",
                    selection.backend
                ));
                NativeBootOutcome::DeliveryFailed {
                    backend: selection.backend,
                    detail,
                }
            }
        };
        self.status.updated_at_ms = now_ms();
        self.persist();
    }

    pub fn record_ports(&mut self, ports: PortsStatus) {
        self.status.ports = Some(ports);
        self.status.updated_at_ms = now_ms();
        self.persist();
    }

    fn persist(&self) {
        let body = Persisted {
            status: self.status.clone(),
            selection: self.selection,
        };
        let Ok(json) = serde_json::to_vec_pretty(&body) else {
            return;
        };
        if let Some(parent) = self.env.state_file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let tmp = self.env.state_file.with_extension("json.tmp");
        if fs::write(&tmp, json)
            .and_then(|()| fs::rename(&tmp, &self.env.state_file))
            .is_err()
        {
            eprintln!("core: native boot state not persisted");
        }
    }

    /// The per-backend activators' state is gone with them: drop their legacy
    /// `load_status` files, and a removed KPM's lock, on the boot scan.
    fn cleanup_legacy(&self, facts: &ScanFacts) {
        let Some(root) = &self.env.legacy_root else {
            return;
        };
        let at = |p: &str| root.join(p.trim_start_matches('/'));
        for file in [
            "/data/adb/vpnhide_kmod/load_status",
            "/data/adb/vpnhide_kmod/load_dmesg",
            "/data/adb/vpnhide_builtin/load_status",
            "/data/adb/vpnhide_kpm/load_status",
        ] {
            let _ = fs::remove_file(at(file));
        }
        let kpm_present = facts.installed.contains(&NativeBackend::Kpm)
            || facts.staged.contains(&NativeBackend::Kpm)
            || facts.kpm == KpmFact::Live;
        if !kpm_present {
            let _ = fs::remove_file(at("/data/adb/vpnhide_kpm/ctl.lock"));
        }
        for dir in [
            "/data/adb/vpnhide_kmod",
            "/data/adb/vpnhide_builtin",
            "/data/adb/vpnhide_kpm",
        ] {
            let _ = fs::remove_dir(at(dir));
        }
    }
}

fn selected_outcome(selection: Selection) -> NativeBootOutcome {
    NativeBootOutcome::Selected {
        backend: selection.backend,
        mode: selection.mode,
        filesystem_hiding: selection.filesystem_hiding,
    }
}

fn read_persisted(path: &Path) -> Option<Persisted> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn load_kmod(
    env: &NativeEnv,
    fs_hiding: bool,
    effects: &dyn Effects,
) -> ((NativeBootOutcome, Option<Selection>), KmodBootReport) {
    let dir = env.module_dir(NativeBackend::Kmod).unwrap_or_default();
    let (kprobes, kretprobes) = effects.kprobes();
    let report = KmodBootReport {
        kprobes,
        kretprobes,
        gki_variant: read_prop(&dir, "gkiVariant"),
        version: read_prop(&dir, "version"),
        uname: effects.uname(),
    };
    let artifact = env.artifact(NativeBackend::Kmod).unwrap_or_default();
    if !artifact.is_file() {
        let outcome = NativeBootOutcome::MissingArtifact {
            backend: NativeBackend::Kmod,
            path: artifact.display().to_string(),
        };
        return ((outcome, None), report);
    }
    let failed = |exit: i32, stderr: String, effects: &dyn Effects| {
        let dmesg = effects.dmesg_tail();
        NativeBootOutcome::LoadFailed {
            backend: NativeBackend::Kmod,
            exit: Some(exit),
            signature_rejected: signature_rejected(exit, &stderr, &dmesg),
            stderr,
            dmesg_tail: dmesg,
            kprobes,
            kretprobes,
        }
    };
    let outcome = match effects.insmod(&env.ctl, &artifact, fs_hiding) {
        Ok(()) => match kernel_fact(&env.ctl) {
            KernelFact::Kmod => {
                let selection = Selection {
                    backend: NativeBackend::Kmod,
                    mode: SelectionMode::Loaded,
                    filesystem_hiding: Some(fs_hiding),
                };
                (selected_outcome(selection), Some(selection))
            }
            other => (
                failed(
                    0,
                    format!("insmod succeeded but the control node reports {other:?}"),
                    effects,
                ),
                None,
            ),
        },
        Err(failure) => (failed(failure.exit, failure.stderr, effects), None),
    };
    (outcome, report)
}

fn load_kpm(
    env: &NativeEnv,
    kpm: Option<&CoreKpm>,
    fs_hiding: bool,
    effects: &dyn Effects,
) -> (NativeBootOutcome, Option<Selection>) {
    let artifact = env.artifact(NativeBackend::Kpm).unwrap_or_default();
    if !artifact.is_file() {
        return (
            NativeBootOutcome::MissingArtifact {
                backend: NativeBackend::Kpm,
                path: artifact.display().to_string(),
            },
            None,
        );
    }
    let load_failed = |stderr: String| NativeBootOutcome::LoadFailed {
        backend: NativeBackend::Kpm,
        exit: None,
        stderr,
        dmesg_tail: Vec::new(),
        kprobes: None,
        kretprobes: None,
        signature_rejected: false,
    };
    let Some(kpm) = kpm else {
        return (
            load_failed(if env.kpm_lock.is_none() {
                "KPM management is not enabled".into()
            } else {
                "no KernelPatch runtime (APatch / KPatch-Next) found".into()
            }),
            None,
        );
    };
    match effects.kpm_kernel() {
        Ok((uname, false)) => {
            return (
                NativeBootOutcome::UnsupportedKernel {
                    backend: NativeBackend::Kpm,
                    uname,
                    detail: format!(
                        "supported families: {}",
                        vpnhide_activator::KPM_SUPPORTED_KERNEL_FAMILIES
                    ),
                },
                None,
            );
        }
        Ok(_) => {}
        Err(e) => return (load_failed(e), None),
    }
    match effects.kpm_load(kpm, &env.ctl, &artifact, fs_hiding) {
        Ok(()) => {
            let selection = Selection {
                backend: NativeBackend::Kpm,
                mode: SelectionMode::Loaded,
                filesystem_hiding: Some(fs_hiding),
            };
            (selected_outcome(selection), Some(selection))
        }
        Err(e) => (load_failed(e), None),
    }
}

/// Replace Zygisk's targets with an empty snapshot.
pub fn retire_zygisk(dir: &Path) -> Result<(), String> {
    let target = crate::zygisk::ZygiskTarget::open(dir)?;
    let wire = crate::activation::project_native_wire_with_resolver(
        &vpnhide_core_rpc::CanonicalConfig::default(),
        NativeBackend::Zygisk,
        &vpnhide_activator::PackageUidMap::default(),
    )?;
    target.publish(&wire).map_err(|e| match e {
        crate::zygisk::PublishError::BeforeWrite(d) | crate::zygisk::PublishError::Unknown(d) => d,
    })
}

#[cfg(test)]
mod tests;
