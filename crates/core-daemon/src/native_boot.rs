//! Once-per-boot native backend selection (core-2.0 §4). Core is the activator:
//! backend modules never load themselves, and the installed set only changes
//! across a reboot (new installs sit in `modules_update`).
//!
//! At startup core scans every native candidate: an installed kmod, KPM or Zygisk
//! module (`module.prop`, no `disable`, no `remove`), the always-live built-in
//! kernel (the control node reports 0x4), a live kmod, and a live KPM core did not
//! load (APatch autoload, or a `.kpm` the user loaded by hand — a supported install
//! method, adopted like any other live backend once its version is verified). Native
//! backends are mutually exclusive:
//!
//! - more than one candidate: load nothing, deliver no config to anything, retire
//!   Zygisk's targets, publish [`NativeBootOutcome::Conflict`];
//! - exactly one: load it (or adopt it when it is already live). An installed
//!   `vpnhide_kpm` module whose KPM is already live is one candidate, not two: core
//!   adopts the live copy and never loads a second one;
//! - none: [`NativeBootOutcome::NoBackend`].
//!
//! CommandEngine journals prepared payload/transition intent before effects. Completed
//! same-boot selections are revalidated on restart. Interrupted or ambiguous loads
//! remain held until explicit RepairActivation can safely adopt a compatible live
//! backend or prepare a new attempt after reboot. KPM credential recovery uses that
//! same durable path; native_boot.json is diagnostic output, never activation authority.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use vpnhide_activator::{CoreKpm, KmodLoadFailure, KpmLiveCheck, KpmLoadFailure, KpmRuntimeProbe};
use vpnhide_core_rpc::{
    KmodBootReport, MAX_DMESG_TAIL, NativeBackend, NativeBootOutcome, NativeBootStatus,
    PortsStatus, SelectionMode, ZygiskTargets,
};

use crate::alog;
pub mod evidence;
pub mod prepared;
pub(crate) mod transition;

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
    /// Legacy inventory root for migration/repair; startup never deletes from it.
    pub legacy_root: Option<PathBuf>,
    /// APatch's installed-KPM dir: evidence that a VPN Hide KPM may be live.
    pub apatch_kpms: PathBuf,
    pub(crate) payload_override: Option<PathBuf>,
    /// Frozen catalog recipient retained through boot selection and later policy apply.
    pub(crate) bundle_identity: Option<prepared::BundleIdentity>,
    pub(crate) loader: Option<std::sync::Arc<crate::zygisk_loader::LoaderRegistry>>,
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
            payload_override: None,
            bundle_identity: None,
            loader: None,
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
        if let Some(path) = &self.payload_override {
            return Some(path.clone());
        }
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
    /// Authenticated; a VPN Hide KPM core can drive is loaded.
    Live,
    /// Authenticated; a VPN Hide KPM is loaded but too old, of an unrecognised
    /// version, or failing the status handshake. Still a live backend (it counts for
    /// conflicts), never configured.
    Incompatible {
        found: Option<String>,
        detail: String,
    },
    /// The runtime refused authentication (no valid superkey / `su` grant).
    AuthFailed(String),
    /// The runtime is present but could not be enumerated.
    Broken(String),
    /// Driver exists without a recognized KPM inventory ABI; no positive KPM evidence.
    Unrecognized(String),
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
    /// The live KPM's load-time filesystem hiding, read back from KernelPatch.
    pub kpm_filesystem_hiding: Option<bool>,
}

impl ScanFacts {
    /// A VPN Hide KPM is loaded, whether or not core can drive it.
    pub fn kpm_live(&self) -> bool {
        matches!(self.kpm, KpmFact::Live | KpmFact::Incompatible { .. })
    }

    pub fn live(&self) -> Vec<NativeBackend> {
        let mut live = Vec::new();
        match self.kernel {
            KernelFact::Builtin => live.push(NativeBackend::Builtin),
            KernelFact::Kmod => live.push(NativeBackend::Kmod),
            _ => {}
        }
        if self.kpm_live() {
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
    if let (KpmFact::Incompatible { found, detail }, [NativeBackend::Kpm]) =
        (&facts.kpm, candidates.as_slice())
    {
        return Plan::Stop(NativeBootOutcome::KpmIncompatible {
            found: found.clone(),
            minimum: vpnhide_activator::MIN_KPM_VERSION.to_owned(),
            detail: detail.clone(),
        });
    }
    if let KpmFact::Unrecognized(detail) = &facts.kpm {
        // Retaining an already-live built-in does not load or expose another hook set.
        // Unknown drivers never authorize a new backend or prove ownership clear.
        if facts.kernel != KernelFact::Builtin
            || facts.kpm_may_be_live
            || candidates.as_slice() != [NativeBackend::Builtin]
        {
            return Plan::Stop(NativeBootOutcome::KpmUnverifiable {
                detail: detail.clone(),
            });
        }
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
        if facts.kpm_may_be_live || !auth {
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
    fn kpm_load(
        &self,
        kpm: &CoreKpm,
        ctl: &Path,
        artifact: &Path,
        fs: bool,
    ) -> Result<(), KpmLoadFailure>;
    /// `(uname -r, the KPM supports it)`.
    fn kpm_kernel(&self) -> Result<(String, bool), String>;
    fn kprobes(&self) -> (Option<bool>, Option<bool>);
    fn dmesg_tail(&self) -> Vec<String>;
    fn uname(&self) -> String;
    fn retire_zygisk(&self, dir: &Path) -> Result<(), String> {
        retire_zygisk(dir)
    }
}

/// The device.
pub struct SystemEffects;

impl Effects for SystemEffects {
    fn insmod(&self, ctl: &Path, artifact: &Path, fs: bool) -> Result<(), KmodLoadFailure> {
        vpnhide_activator::insmod_selected_kmod(ctl, artifact, fs)
    }

    fn kpm_load(
        &self,
        kpm: &CoreKpm,
        ctl: &Path,
        artifact: &Path,
        fs: bool,
    ) -> Result<(), KpmLoadFailure> {
        kpm.load_selected(ctl, artifact, fs)
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

pub(crate) fn read_boot_id(path: &Path) -> String {
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

/// What [`probe_kpm`] learned: the fact, the authenticated channel (when core may use
/// it), and a live KPM's load-time filesystem hiding read back from KernelPatch.
pub struct KpmProbe {
    pub fact: KpmFact,
    pub client: Option<CoreKpm>,
    pub filesystem_hiding: Option<bool>,
}

/// Probe KernelPatch through core's management channel. A live `vpnhide` KPM is
/// identified (status handshake + version) whoever loaded it.
pub fn probe_kpm(lock: Option<&Path>) -> KpmProbe {
    let (fact, client, filesystem_hiding) = probe_kpm_parts(lock);
    KpmProbe {
        fact,
        client,
        filesystem_hiding,
    }
}

fn probe_kpm_parts(lock: Option<&Path>) -> (KpmFact, Option<CoreKpm>, Option<bool>) {
    let Some(lock) = lock else {
        // Unmanaged KPM is never loaded or configured, but a live one still counts.
        return match CoreKpm::observe_loaded() {
            Ok(vpnhide_activator::KpmObserved::Loaded) => (KpmFact::Live, None, None),
            Ok(vpnhide_activator::KpmObserved::Clear) => (KpmFact::NotManaged, None, None),
            Ok(vpnhide_activator::KpmObserved::Unrecognized(detail)) => {
                (KpmFact::Unrecognized(detail), None, None)
            }
            Err(e) => (KpmFact::Broken(e.to_string()), None, None),
        };
    };
    match CoreKpm::probe(lock) {
        Ok(KpmRuntimeProbe::NoRuntime) => (KpmFact::NoRuntime, None, None),
        Ok(KpmRuntimeProbe::Unrecognized(detail)) => (KpmFact::Unrecognized(detail), None, None),
        Ok(KpmRuntimeProbe::InventoryOnly {
            loaded: true,
            detail,
        }) => (
            KpmFact::Incompatible {
                found: None,
                detail,
            },
            None,
            None,
        ),
        Ok(KpmRuntimeProbe::InventoryOnly { loaded: false, .. }) => {
            (KpmFact::NotManaged, None, None)
        }
        Ok(KpmRuntimeProbe::AwaitingAuthentication(detail)) => {
            (KpmFact::AuthFailed(detail), None, None)
        }
        Ok(KpmRuntimeProbe::Ready(client)) => match client.verify_live() {
            Ok(KpmLiveCheck::Compatible { info }) => {
                let fs = info.as_ref().map(|i| i.filesystem_hiding());
                alog::info(&format!(
                    "native: live vpnhide KPM version {} (filesystem_hiding={fs:?})",
                    info.as_ref().map_or("unreported", |i| i.version.as_str()),
                ));
                (KpmFact::Live, Some(client), fs)
            }
            Ok(KpmLiveCheck::NotLoaded) => (KpmFact::Absent, Some(client), None),
            Ok(KpmLiveCheck::Incompatible { found, detail }) => {
                alog::info(&format!(
                    "native: live vpnhide KPM is incompatible: {detail}"
                ));
                (KpmFact::Incompatible { found, detail }, None, None)
            }
            Err(e) => (KpmFact::Broken(e.to_string()), None, None),
        },
        Err(e) => (KpmFact::Broken(e.to_string()), None, None),
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
/// 1.x companion modules 2.0 replaced with core. One still installed keeps running its
/// own boot scripts beside core (iptables `vpnhide_out`, `/proc/vpnhide_ctl`). A module
/// already marked for removal (`remove`) is gone after the next reboot and is not listed.
pub const OBSOLETE_MODULE_IDS: [&str; 2] = ["vpnhide_ports", "vpnhide_builtin"];

/// The [`OBSOLETE_MODULE_IDS`] still installed under `modules_root`.
pub fn obsolete_modules(modules_root: &Path) -> Vec<String> {
    OBSOLETE_MODULE_IDS
        .into_iter()
        .filter(|id| {
            let dir = modules_root.join(id);
            dir.is_dir() && !dir.join("remove").exists()
        })
        .map(str::to_owned)
        .collect()
}

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
        kpm_filesystem_hiding: None,
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
    /// Publish an activation hold without scanning, loading, retiring or cleaning.
    pub(crate) fn blocked(env: NativeEnv, epoch: u64, detail: String) -> Self {
        let now = now_ms();
        Self {
            status: NativeBootStatus {
                boot_id: read_boot_id(&env.boot_id_path),
                decided_epoch: epoch,
                decided_at_ms: now,
                updated_at_ms: now,
                outcome: NativeBootOutcome::Unknown { detail },
                installed: Vec::new(),
                live: Vec::new(),
                staged: Vec::new(),
                kmod: None,
                zygisk_targets: ZygiskTargets::Pending,
                ports: None,
                obsolete_modules: Vec::new(),
            },
            env,
            selection: None,
        }
    }

    /// Recheck residual ownership without loading, configuring or retiring anything.
    /// A durable selection is not proof that its runtime still exists.
    pub(crate) fn revalidate_restored(self, epoch: u64) -> Self {
        let Some(selected) = self.selected() else {
            return self;
        };
        let probe = probe_kpm(self.env.kpm_lock.as_deref());
        let facts = gather(&self.env, probe.fact);
        self.reconcile_restored_facts(epoch, selected, &facts)
    }

    fn reconcile_restored_facts(
        mut self,
        epoch: u64,
        selected: NativeBackend,
        facts: &ScanFacts,
    ) -> Self {
        if selected == NativeBackend::Kpm
            && let KpmFact::AuthFailed(detail) = &facts.kpm
        {
            let mut boot = Self::blocked(self.env, epoch, detail.clone());
            // Preserve the existing credential recovery path. This is evidence
            // of a previously selected KPM, never permission to load an alternative.
            boot.status.outcome = NativeBootOutcome::KpmUnverifiable {
                detail: detail.clone(),
            };
            return boot;
        }
        let kpm_clear = matches!(
            facts.kpm,
            KpmFact::NotManaged | KpmFact::NoRuntime | KpmFact::Absent
        ) || (!facts.kpm_may_be_live
            && matches!(facts.kpm, KpmFact::AuthFailed(_)));
        let valid = match selected {
            NativeBackend::Builtin => {
                facts.kernel == KernelFact::Builtin
                    && (kpm_clear
                        || (matches!(facts.kpm, KpmFact::Unrecognized(_))
                            && !facts.kpm_may_be_live
                            && facts.installed.is_empty()))
            }
            NativeBackend::Kmod => facts.kernel == KernelFact::Kmod && kpm_clear,
            NativeBackend::Kpm => facts.kernel == KernelFact::Absent && facts.kpm == KpmFact::Live,
            NativeBackend::Zygisk => {
                facts.kernel == KernelFact::Absent
                    && kpm_clear
                    && self.env.module_dir(selected).is_some_and(|dir| {
                        let Ok(target) = crate::zygisk::ZygiskTarget::open(&dir) else {
                            return false;
                        };
                        match target.bundled() {
                            Ok(false) => true, // explicit separate-module compatibility
                            Ok(true) => crate::payloads::Catalog::open(&dir).is_ok_and(|catalog| {
                                let abi = if cfg!(target_arch = "aarch64") {
                                    crate::payloads::Abi::Arm64
                                } else {
                                    crate::payloads::Abi::Arm32
                                };
                                let needs32 = crate::payloads::device::needs_arm32_zygisk(
                                    Path::new("/proc"),
                                    vpnhide_activator::sys::property("ro.zygote").as_deref(),
                                );
                                self.env
                                    .loader
                                    .as_ref()
                                    .is_some_and(|loader| loader.available(&catalog, abi, needs32))
                            }),
                            Err(_) => false,
                        }
                    })
            }
        };
        if valid {
            if selected == NativeBackend::Zygisk
                && self.env.module_dir(selected).is_some_and(|dir| {
                    crate::zygisk::ZygiskTarget::open(&dir).and_then(|t| t.is_bundled()) == Ok(true)
                })
            {
                if let Some(selection) = self.selection.as_mut() {
                    selection.mode = SelectionMode::Authorized;
                }
                if let NativeBootOutcome::Selected { mode, .. } = &mut self.status.outcome {
                    *mode = SelectionMode::Authorized;
                }
            }
            self
        } else {
            Self::blocked(
                self.env,
                epoch,
                format!(
                    "durable {selected:?} selection cannot be reconciled with current runtime ownership; no load replayed"
                ),
            )
        }
    }

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

    /// Decide and perform the load / retire for `facts`.
    #[cfg(test)]
    pub fn decide_and_apply(
        env: NativeEnv,
        epoch: u64,
        boot_id: &str,
        facts: &ScanFacts,
        kpm: Option<CoreKpm>,
        filesystem_hiding: bool,
        effects: &dyn Effects,
    ) -> Self {
        Self::apply_prepared_plan(
            env,
            epoch,
            boot_id,
            facts,
            kpm,
            filesystem_hiding,
            effects,
            decide(facts),
        )
    }

    /// Execute only the decision that preparation bound to durable intent. Facts
    /// remain diagnostic evidence and cannot cause a second selection here.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_prepared_plan(
        env: NativeEnv,
        epoch: u64,
        boot_id: &str,
        facts: &ScanFacts,
        kpm: Option<CoreKpm>,
        filesystem_hiding: bool,
        effects: &dyn Effects,
        plan: Plan,
    ) -> Self {
        let mut kmod_report = None;
        let (outcome, selection) = match plan {
            Plan::Stop(outcome) => (outcome, None),
            Plan::Adopt(backend) => {
                let selection = Selection {
                    backend,
                    mode: if backend == NativeBackend::Zygisk
                        && env.module_dir(backend).is_some_and(|dir| {
                            crate::zygisk::ZygiskTarget::open(&dir).and_then(|t| t.is_bundled())
                                == Ok(true)
                        }) {
                        SelectionMode::Authorized
                    } else {
                        SelectionMode::Adopted
                    },
                    filesystem_hiding: (backend == NativeBackend::Kpm)
                        .then_some(facts.kpm_filesystem_hiding)
                        .flatten(),
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
                obsolete_modules: env
                    .modules
                    .as_deref()
                    .map(obsolete_modules)
                    .unwrap_or_default(),
            },
            selection,
            env,
        };
        boot.status.zygisk_targets = boot.settle_zygisk(facts, effects);
        if !boot.status.obsolete_modules.is_empty() {
            alog::info(&format!(
                "native: obsolete 1.x modules installed: {}; their boot scripts still run beside core (iptables, /proc/vpnhide_ctl). Uninstall them and reboot",
                boot.status.obsolete_modules.join(", ")
            ));
        }
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
    fn settle_zygisk(&self, facts: &ScanFacts, effects: &dyn Effects) -> ZygiskTargets {
        if !facts.installed.contains(&NativeBackend::Zygisk) {
            return ZygiskTargets::NotInstalled;
        }
        if self.selected() == Some(NativeBackend::Zygisk) {
            return ZygiskTargets::Pending;
        }
        let Some(dir) = self.env.module_dir(NativeBackend::Zygisk) else {
            return ZygiskTargets::NotInstalled;
        };
        match effects.retire_zygisk(&dir) {
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

    /// The config no longer fits the selected backend (Repair or boot delivery). Nothing
    /// was written: the backend keeps whatever it held, and the status names the packages
    /// to deselect instead of reporting a generic delivery failure.
    pub fn record_capacity_exceeded(&mut self, report: vpnhide_core_rpc::NativeCapacityReport) {
        let Some(selection) = self.selection else {
            return;
        };
        alog::info(&format!(
            "native: refused config for {:?}: {} target uids exceed capacity {}; nothing written",
            selection.backend, report.total, report.capacity
        ));
        self.status.outcome = NativeBootOutcome::CapacityExceeded {
            backend: selection.backend,
            report,
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
}

fn selected_outcome(selection: Selection) -> NativeBootOutcome {
    NativeBootOutcome::Selected {
        backend: selection.backend,
        mode: selection.mode,
        filesystem_hiding: selection.filesystem_hiding,
    }
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
        Err(e) => (load_failed(e.to_string()), None),
    }
}

/// Replace Zygisk's targets with an empty snapshot.
pub fn retire_zygisk(dir: &Path) -> Result<(), String> {
    let target = crate::zygisk::ZygiskTarget::open(dir)?;
    retire_zygisk_target(&target)
}

pub(crate) fn retire_zygisk_target(target: &crate::zygisk::ZygiskTarget) -> Result<(), String> {
    if target.is_bundled()? {
        return if target.authorization_absent()? {
            Ok(())
        } else {
            Err("bundled authorization requires writer-owned journaled revocation".into())
        };
    }
    let wire = crate::activation::project_native_wire_with_resolver(
        &vpnhide_core_rpc::CanonicalConfig::default(),
        NativeBackend::Zygisk,
        &vpnhide_activator::PackageUidMap::default(),
    )
    .map_err(|e| e.to_string())?;
    target.publish(&wire).map_err(|e| match e {
        crate::zygisk::PublishError::BeforeWrite(d) | crate::zygisk::PublishError::Unknown(d) => d,
    })
}

#[cfg(test)]
mod tests;
