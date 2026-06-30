use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::error::Error;
use std::ffi::CString;
use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::os::raw::{c_char, c_long, c_void};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use std::ptr;
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use vpnhide_protocol::Target;
use vpnhide_protocol::hook_ids::{HOOK_NAMES, KERNEL_HOOK_MASK, ZYGISK_HOOK_MASK};
use vpnhide_protocol::{MAX_TARGET_UIDS, format_config, parse_config};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

pub const CANONICAL_CONFIG: &str = "/data/system/vpnhide_config.json";
pub const KMOD_CTL: &str = "/proc/vpnhide_ctl";
pub const KMOD_MODULE_DIR: &str = "/data/adb/modules/vpnhide_kmod";
pub const ZYGISK_RUNTIME_CONFIG: &str = "/data/adb/modules/vpnhide_zygisk/targets.txt";
pub const KPM_MODULE_FILE: &str = "/data/adb/modules/vpnhide_kpm/vpnhide.kpm";
pub const SUPERKEY_FILE: &str = "/data/adb/vpnhide/superkey";
const APATCH_DIR: &str = "/data/adb/ap";
const APP_PACKAGE: &str = "dev.okhsunrog.vpnhide";
const KPM_NAME: &str = "vpnhide";
const PORTS_CHAIN4: &str = "vpnhide_out";
const PORTS_CHAIN6: &str = "vpnhide_out6";
// The native-target cap is owned by the shared protocol crate (and mirrored by
// the C backends' `#define MAX_TARGET_UIDS`); alias it here so all three stay in
// lock-step instead of restating the literal 64.
const MAX_NATIVE_TARGETS: usize = MAX_TARGET_UIDS;
const PM_READY_ATTEMPTS: u32 = 60;
const APATCH_SUPERCALL_NR: c_long = 45;
const APATCH_SUPERCALL_DEFAULT_VERSION_CODE: c_long = 0x000d00;
const APATCH_SUPERCALL_MAGIC: c_long = 0x1158;
const SUPERCALL_HELLO: c_long = 0x1000;
const SUPERCALL_HELLO_MAGIC: c_long = 0x11581158;
const SUPERCALL_KPM_LOAD: c_long = 0x1020;
const SUPERCALL_KPM_CONTROL: c_long = 0x1022;
const SUPERCALL_KPM_LIST: c_long = 0x1031;
const APATCH_SUPERCALL_VERSION_FALLBACKS: &[c_long] =
    &[0x000c02, 0x000c01, 0x000c00, 0x000b01, 0x000b00, 0x000a05];

unsafe extern "C" {
    fn syscall(num: c_long, ...) -> c_long;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApatchCommandStyle {
    Versioned(c_long),
    Raw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PmReadyWait {
    Bounded(u32),
    Forever,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalConfig {
    #[serde(default = "schema_version")]
    pub version: u32,
    #[serde(default)]
    pub debug: bool,
    #[serde(default)]
    pub apps: BTreeMap<String, AppConfig>,
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub remember_superkey: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    // `java` selects LSPosed (system_server) hooks and is owned entirely by the
    // LSPosed self-read path; the native activator never inspects it. It must
    // still *parse*: the app writes it as a bool (all / none) or — for a
    // per-hook Java selection — a JSON array of hook names, the same shape as
    // `native`. Accept both so a partial Java selection never breaks the native
    // config read (a bool-only field would error on the array form).
    #[serde(default, deserialize_with = "de_bool_or_hook_list")]
    pub java: bool,
    #[serde(default)]
    pub native: NativeSelection,
    #[serde(default)]
    pub app_hiding: bool,
    #[serde(default)]
    pub ports: bool,
    #[serde(default)]
    pub port_policy: Option<PortPolicy>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortPolicy {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub rules: Vec<PortRule>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub struct PortRule {
    pub start: u16,
    #[serde(default)]
    pub end: Option<u16>,
    #[serde(default = "default_port_protocol")]
    pub protocol: PortProtocol,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Both,
    Tcp,
    Udp,
}

impl PortRule {
    fn end_port(self) -> u16 {
        self.end.unwrap_or(self.start)
    }

    fn normalized(self) -> Self {
        if self.end_port() == self.start {
            Self { end: None, ..self }
        } else {
            self
        }
    }
}

fn default_port_protocol() -> PortProtocol {
    PortProtocol::Both
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum NativeSelection {
    Enabled(bool),
    Hooks(Vec<String>),
    Detailed(NativeSelectionDetail),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeSelectionDetail {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub kernel: Option<Vec<String>>,
    #[serde(default)]
    pub zygisk: Option<Vec<String>>,
}

impl Default for NativeSelection {
    fn default() -> Self {
        Self::Enabled(false)
    }
}

fn default_enabled() -> bool {
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeHookFamily {
    Kernel,
    Zygisk,
}

impl NativeHookFamily {
    fn full_mask(self) -> u32 {
        match self {
            NativeHookFamily::Kernel => KERNEL_HOOK_MASK,
            NativeHookFamily::Zygisk => ZYGISK_HOOK_MASK,
        }
    }
}

impl NativeSelection {
    fn hookmask(&self, family: NativeHookFamily) -> Option<u32> {
        match self {
            NativeSelection::Enabled(false) => None,
            NativeSelection::Enabled(true) => Some(family.full_mask()),
            NativeSelection::Hooks(names) => {
                if names.is_empty() {
                    return None;
                }
                let mask = match family {
                    NativeHookFamily::Kernel => {
                        names.iter().fold(0u32, |acc, name| acc | hook_bit(name)) & KERNEL_HOOK_MASK
                    }
                    NativeHookFamily::Zygisk => ZYGISK_HOOK_MASK,
                };
                (mask != 0).then_some(mask)
            }
            NativeSelection::Detailed(detail) => {
                if !detail.enabled {
                    return None;
                }
                let selected = match family {
                    NativeHookFamily::Kernel => &detail.kernel,
                    NativeHookFamily::Zygisk => &detail.zygisk,
                };
                let Some(names) = selected else {
                    return Some(family.full_mask());
                };
                let mask =
                    names.iter().fold(0u32, |acc, name| acc | hook_bit(name)) & family.full_mask();
                (mask != 0).then_some(mask)
            }
        }
    }
}

fn schema_version() -> u32 {
    1
}

/// Deserialize a hook-role field that the app writes as either a bool (all /
/// none) or a JSON array of hook names (partial selection), collapsing it to
/// "is this role enabled". Used for `java`, which the native activator parses
/// but does not act on — the LSPosed self-read path owns Java hook selection.
fn de_bool_or_hook_list<'de, D>(deserializer: D) -> std::result::Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BoolOrHookList {
        Bool(bool),
        Hooks(Vec<String>),
    }
    Ok(match BoolOrHookList::deserialize(deserializer)? {
        BoolOrHookList::Bool(enabled) => enabled,
        BoolOrHookList::Hooks(hooks) => !hooks.is_empty(),
    })
}

fn hook_bit(name: &str) -> u32 {
    HOOK_NAMES
        .iter()
        .position(|known| *known == name)
        .map(|id| 1u32 << id)
        .unwrap_or(0)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageUidMap {
    packages: BTreeMap<String, Vec<u32>>,
}

impl PackageUidMap {
    pub fn from_pm() -> Result<Self> {
        Self::from_pm_with_wait(PmReadyWait::Bounded(PM_READY_ATTEMPTS))
    }

    fn from_pm_with_wait(wait: PmReadyWait) -> Result<Self> {
        wait_for_pm_ready(wait)?;
        let stdout = pm_list_packages(&["list", "packages", "-U", "--user", "all"])?;
        Ok(parse_pm_packages(&stdout))
    }

    fn uids_for(&self, package: &str) -> &[u32] {
        self.packages.get(package).map(Vec::as_slice).unwrap_or(&[])
    }
}

pub fn parse_pm_packages(output: &str) -> PackageUidMap {
    let mut packages = BTreeMap::<String, Vec<u32>>::new();
    for line in output.lines() {
        let mut pkg: Option<&str> = None;
        let mut uid_csv: Option<&str> = None;
        for token in line.split_whitespace() {
            if let Some(rest) = token.strip_prefix("package:") {
                pkg = Some(rest);
            } else if let Some(rest) = token.strip_prefix("uid:") {
                uid_csv = Some(rest);
            }
        }
        let (Some(pkg), Some(uid_csv)) = (pkg, uid_csv) else {
            continue;
        };
        let uids = uid_csv
            .split(',')
            .filter_map(|s| s.parse::<u32>().ok())
            .collect::<Vec<_>>();
        if !uids.is_empty() {
            packages.insert(pkg.to_owned(), uids);
        }
    }
    PackageUidMap { packages }
}

pub fn parse_canonical(json: &str) -> Result<CanonicalConfig> {
    let cfg: CanonicalConfig = serde_json::from_str(json)?;
    if cfg.version > schema_version() {
        return Err(format!("unsupported vpnhide config version {}", cfg.version).into());
    }
    validate_port_policies(&cfg)?;
    Ok(cfg)
}

fn validate_port_policies(cfg: &CanonicalConfig) -> Result<()> {
    for (pkg, app) in &cfg.apps {
        let Some(policy) = &app.port_policy else {
            continue;
        };
        if policy.rules.is_empty() {
            return Err(format!("{pkg}: portPolicy.rules must not be empty").into());
        }
        for rule in &policy.rules {
            let end = rule.end_port();
            if rule.start == 0 || end == 0 {
                return Err(format!("{pkg}: port ranges must be within 1..65535").into());
            }
            if rule.start > end {
                return Err(format!("{pkg}: port range start must not exceed end").into());
            }
        }
    }
    Ok(())
}

pub fn project_native(json: &str) -> Result<String> {
    project_native_with_pm_wait(
        json,
        NativeHookFamily::Kernel,
        PmReadyWait::Bounded(PM_READY_ATTEMPTS),
    )
}

fn project_native_with_pm_wait(
    json: &str,
    family: NativeHookFamily,
    wait: PmReadyWait,
) -> Result<String> {
    let cfg = parse_canonical(json)?;
    if !has_native_targets(&cfg, family) {
        return Ok(format_config(cfg.debug, &[]));
    }
    let resolver = PackageUidMap::from_pm_with_wait(wait)?;
    Ok(project_native_with_resolver_for_family(
        &cfg, &resolver, family,
    ))
}

fn project_native_with_resolver_for_family(
    cfg: &CanonicalConfig,
    resolver: &PackageUidMap,
    family: NativeHookFamily,
) -> String {
    let mut by_uid = BTreeMap::<u32, u32>::new();
    for (pkg, app) in &cfg.apps {
        let Some(mask) = app.native.hookmask(family) else {
            continue;
        };
        for uid in resolver.uids_for(pkg) {
            by_uid
                .entry(*uid)
                .and_modify(|existing| *existing |= mask)
                .or_insert(mask);
        }
    }
    // `by_uid` is a BTreeMap, so iteration is ascending by UID; truncating would
    // silently drop the highest-UID (typically most-recently-installed) apps with
    // no diagnostic. Warn so a user with more native targets than the backend can
    // hold learns their protection is partial, instead of failing closed silently.
    if by_uid.len() > MAX_NATIVE_TARGETS {
        eprintln!(
            "vpnhide: WARNING: {} native targets exceed the backend cap of {}; \
             dropping the {} highest-UID app(s) from native protection",
            by_uid.len(),
            MAX_NATIVE_TARGETS,
            by_uid.len() - MAX_NATIVE_TARGETS,
        );
    }
    let targets = by_uid
        .into_iter()
        .take(MAX_NATIVE_TARGETS)
        .map(|(uid, hookmask)| Target { uid, hookmask })
        .collect::<Vec<_>>();
    format_config(cfg.debug, &targets)
}

pub fn project_native_with_resolver(cfg: &CanonicalConfig, resolver: &PackageUidMap) -> String {
    project_native_with_resolver_for_family(cfg, resolver, NativeHookFamily::Kernel)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortsRuleset {
    pub ipv4: String,
    pub ipv6: String,
    pub target_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PortUidPolicy {
    all_ports: bool,
    rules: BTreeSet<PortRule>,
}

impl PortUidPolicy {
    fn merge_app(&mut self, app: &AppConfig) {
        if self.all_ports {
            return;
        }
        let Some(policy) = &app.port_policy else {
            self.all_ports = true;
            self.rules.clear();
            return;
        };
        self.rules
            .extend(policy.rules.iter().copied().map(PortRule::normalized));
    }
}

pub fn project_ports(json: &str) -> Result<PortsRuleset> {
    project_ports_with_pm_wait(json, PmReadyWait::Bounded(PM_READY_ATTEMPTS))
}

fn project_ports_with_pm_wait(json: &str, wait: PmReadyWait) -> Result<PortsRuleset> {
    let cfg = parse_canonical(json)?;
    if !has_ports_targets(&cfg) {
        return Ok(project_ports_with_resolver(&cfg, &PackageUidMap::default()));
    }
    let resolver = PackageUidMap::from_pm_with_wait(wait)?;
    Ok(project_ports_with_resolver(&cfg, &resolver))
}

pub fn project_ports_with_resolver(
    cfg: &CanonicalConfig,
    resolver: &PackageUidMap,
) -> PortsRuleset {
    let mut targets = BTreeMap::<u32, PortUidPolicy>::new();
    for (pkg, app) in &cfg.apps {
        if !app.ports {
            continue;
        }
        for uid in resolver.uids_for(pkg) {
            if *uid >= 10_000 {
                targets.entry(*uid).or_default().merge_app(app);
            }
        }
    }
    PortsRuleset {
        // Match the whole IPv4 loopback block, not just 127.0.0.1: a localhost
        // proxy/VPN daemon bound to the wildcard 0.0.0.0 (the common allow-lan /
        // TUN config for Clash, sing-box, V2Ray) is reachable on every 127.x.x.x
        // alias, so an observer could `connect(127.0.0.2:port)` and still get a
        // handshake — a positive fingerprint — if only 127.0.0.1 were rejected.
        // (::1 already is the entire IPv6 loopback.)
        ipv4: build_ports_ruleset(PORTS_CHAIN4, "127.0.0.0/8", "icmp-port-unreachable", &targets),
        ipv6: build_ports_ruleset(PORTS_CHAIN6, "::1", "icmp6-port-unreachable", &targets),
        target_count: targets.len(),
    }
}

pub fn read_canonical() -> Result<String> {
    match fs::read_to_string(CANONICAL_CONFIG) {
        Ok(raw) => Ok(raw),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(empty_canonical_json().to_owned()),
        Err(e) => Err(e.into()),
    }
}

pub fn activate_kmod() -> Result<()> {
    activate_kmod_with_pm_wait(PmReadyWait::Bounded(PM_READY_ATTEMPTS))
}

pub fn activate_kmod_boot() -> Result<()> {
    wait_for_path(KMOD_CTL);
    activate_kmod_with_pm_wait(PmReadyWait::Forever)
}

fn activate_kmod_with_pm_wait(wait: PmReadyWait) -> Result<()> {
    let wire = project_native_with_pm_wait(&read_canonical()?, NativeHookFamily::Kernel, wait)?;
    // /proc/vpnhide_ctl replaces the entire config per write(), so keep this
    // bounded to MAX_NATIVE_TARGETS and deliver one complete snapshot.
    fs::write(KMOD_CTL, wire)?;
    Ok(())
}

pub fn activate_zygisk() -> Result<()> {
    activate_zygisk_with_pm_wait(PmReadyWait::Bounded(PM_READY_ATTEMPTS))
}

pub fn activate_zygisk_boot() -> Result<()> {
    activate_zygisk_with_pm_wait(PmReadyWait::Forever)
}

fn activate_zygisk_with_pm_wait(wait: PmReadyWait) -> Result<()> {
    let wire = project_native_with_pm_wait(&read_canonical()?, NativeHookFamily::Zygisk, wait)?;
    write_atomic(Path::new(ZYGISK_RUNTIME_CONFIG), wire.as_bytes(), 0o644)
}

/// Outcome of a KPM boot activation. A kmod conflict is a legitimate,
/// non-error result (the KPM deliberately stands down — see §1.5), so it must
/// be distinguishable from a successful configure: the boot script reports
/// each as a different `load_status`, and reporting a deferral as "configured"
/// would lie to the diagnostics screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KpmBootOutcome {
    /// Wire snapshot was delivered to the KPM over ctl0.
    Configured,
    /// The .ko backend is present, so the KPM stood down without loading or
    /// configuring (co-residence freezes the kernel — protocol §1.5).
    DeferredConflict,
}

pub fn activate_kpm() -> Result<()> {
    // App / manual path: a conflict is a hard error (conflict_is_error=true),
    // so this only ever returns Configured on success.
    activate_kpm_with_pm_wait(PmReadyWait::Bounded(PM_READY_ATTEMPTS), true).map(|_| ())
}

pub fn activate_kpm_boot() -> Result<KpmBootOutcome> {
    activate_kpm_with_pm_wait(PmReadyWait::Forever, false)
}

pub fn read_kpm_status() -> Result<String> {
    read_kpm_payload("vpnhide 1 status")
}

pub fn read_kpm_stats() -> Result<String> {
    read_kpm_payload("vpnhide 1 stats")
}

pub fn read_kpm_state() -> Result<String> {
    let client = KpmClient::detect()?;
    let mut out = client.ctl0_read("vpnhide 1 status")?;
    out.push_str(&client.ctl0_read("vpnhide 1 stats")?);
    Ok(out)
}

fn read_kpm_payload(wire: &str) -> Result<String> {
    let client = KpmClient::detect()?;
    client.ctl0_read(wire)
}

fn activate_kpm_with_pm_wait(wait: PmReadyWait, conflict_is_error: bool) -> Result<KpmBootOutcome> {
    if skip_kpm_for_kmod_conflict(conflict_is_error)? {
        return Ok(KpmBootOutcome::DeferredConflict);
    }
    let wire = project_native_with_pm_wait(&read_canonical()?, NativeHookFamily::Kernel, wait)?;
    // Re-check after the (possibly long) PackageManager wait: the .ko may have
    // been loaded meanwhile, in which case we must not configure the KPM.
    if skip_kpm_for_kmod_conflict(conflict_is_error)? {
        return Ok(KpmBootOutcome::DeferredConflict);
    }
    let client = KpmClient::detect()?;
    client.ensure_loaded()?;
    client.ctl0_config(&wire)?;
    Ok(KpmBootOutcome::Configured)
}

pub fn activate_ports() -> Result<()> {
    activate_ports_with_pm_wait(PmReadyWait::Bounded(PM_READY_ATTEMPTS))
}

pub fn activate_ports_boot() -> Result<()> {
    activate_ports_with_pm_wait(PmReadyWait::Forever)
}

fn activate_ports_with_pm_wait(wait: PmReadyWait) -> Result<()> {
    let rules = project_ports_with_pm_wait(&read_canonical()?, wait)?;
    apply_ports_rules(&rules)
}

pub fn boot_wait_requested_from_env() -> Result<bool> {
    let mut boot_wait = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--boot-wait" => boot_wait = true,
            _ => {
                return Err(
                    format!("unknown argument {arg}; usage: activator [--boot-wait]").into(),
                );
            }
        }
    }
    Ok(boot_wait)
}

fn has_native_targets(cfg: &CanonicalConfig, family: NativeHookFamily) -> bool {
    cfg.apps
        .values()
        .any(|app| app.native.hookmask(family).is_some())
}

fn has_ports_targets(cfg: &CanonicalConfig) -> bool {
    cfg.apps.values().any(|app| app.ports)
}

fn empty_canonical_json() -> &'static str {
    "{\"version\":1,\"debug\":false,\"apps\":{},\"settings\":{\"rememberSuperkey\":false}}\n"
}

fn wait_for_pm_ready(wait: PmReadyWait) -> Result<()> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        if let Ok(stdout) = pm_list_packages(&["list", "packages", "-U"])
            && pm_output_has_package(&stdout, APP_PACKAGE)
        {
            return Ok(());
        }
        if matches!(wait, PmReadyWait::Bounded(max) if attempts >= max) {
            return Err(
                format!("PackageManager did not expose {APP_PACKAGE} within {attempts}s").into(),
            );
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn wait_for_path(path: &str) {
    while !Path::new(path).exists() {
        thread::sleep(Duration::from_secs(1));
    }
}

fn pm_list_packages(args: &[&str]) -> Result<String> {
    let out = Command::new("pm").args(args).output()?;
    if !out.status.success() {
        return Err(format!("pm list packages failed with status {}", out.status).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn pm_output_has_package(output: &str, package: &str) -> bool {
    let expected = format!("package:{package}");
    output
        .lines()
        .any(|line| line.split_whitespace().next() == Some(expected.as_str()))
}

/// True when the .ko backend is present and not disabled. This is the most
/// complete of the project's "is the kmod here?" checks: it catches both an
/// installed-and-enabled module directory *and* a live `/proc/vpnhide_ctl`
/// (e.g. a manually-loaded .ko whose module dir is gone). The boot scripts
/// keep a cheaper directory-only check as a fail-safe floor (it cannot error
/// and is ordering-independent); this superset is the authoritative gate on
/// the config-delivery path. See protocol §1.5.
pub fn kmod_backend_present() -> bool {
    Path::new(KMOD_CTL).exists()
        || (Path::new(KMOD_MODULE_DIR).is_dir()
            && !Path::new(KMOD_MODULE_DIR).join("disable").exists())
}

fn skip_kpm_for_kmod_conflict(conflict_is_error: bool) -> Result<bool> {
    if !kmod_backend_present() {
        return Ok(false);
    }
    // .ko present. App/manual path treats this as a hard error; boot path
    // signals a (non-error) deferral to the caller.
    if conflict_is_error {
        return Err("kmod backend present; refusing to load/configure KPM".into());
    }
    Ok(true)
}

pub fn write_atomic(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().ok_or("path has no parent")?;
    fs::create_dir_all(parent)?;
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(content)?;
        file.sync_all()?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    fs::rename(&tmp, path)?;
    Ok(())
}

// Locate the KPatch-Next `kpatch` CLI. This is the KPatch-Next-Module path only
// (Magisk / KSU / KSU-Next all install the same module — bin path confirmed on a
// Pixel 8 Pro). APatch is NOT covered here: it has no kpatch CLI on disk (the
// binary lives in the manager app's private libs) and loads KPMs via the
// supercall instead, so KpmClient::detect() routes APatch through that path.
fn find_kpatch() -> Option<PathBuf> {
    [
        "kpatch",
        "/data/adb/modules/KPatch-Next/bin/kpatch",
        "/data/adb/modules/kpatch-next/bin/kpatch",
    ]
    .into_iter()
    .find_map(|candidate| {
        if candidate.contains('/') {
            let p = PathBuf::from(candidate);
            p.is_file().then_some(p)
        } else {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|dir| dir.join(candidate))
                    .find(|path| path.is_file())
            })
        }
    })
}

fn read_superkey() -> Result<String> {
    let key = fs::read_to_string(SUPERKEY_FILE)?;
    let key = key.trim().to_owned();
    if key.is_empty() {
        Err("superkey file is empty".into())
    } else {
        Ok(key)
    }
}

enum KpmClient {
    KpatchCli {
        path: PathBuf,
    },
    ApatchSupercall {
        key: String,
        style: ApatchCommandStyle,
    },
}

impl KpmClient {
    fn detect() -> Result<Self> {
        if Path::new(APATCH_DIR).is_dir() {
            let key = read_superkey()
                .map_err(|e| format!("APatch KPM requires saved superkey at {SUPERKEY_FILE}: {e}"));
            let key = key?;
            let style = apatch_probe(&key)?;
            return Ok(Self::ApatchSupercall { key, style });
        }
        let path = find_kpatch().ok_or("kpatch CLI not found")?;
        kpatch_hello(&path)?;
        Ok(Self::KpatchCli { path })
    }

    fn ensure_loaded(&self) -> Result<()> {
        if self.list_contains()? {
            return Ok(());
        }
        if !Path::new(KPM_MODULE_FILE).is_file() {
            return Err(format!("{KPM_MODULE_FILE} not found").into());
        }
        self.load()?;
        if self.list_contains()? {
            Ok(())
        } else {
            Err("kpm load returned success but vpnhide is not listed".into())
        }
    }

    fn list_contains(&self) -> Result<bool> {
        match self {
            Self::KpatchCli { path } => kpatch_kpm_list_contains(path),
            Self::ApatchSupercall { key, style } => {
                let list = apatch_kpm_list(key, *style)?;
                Ok(list.split_whitespace().any(|token| token == KPM_NAME))
            }
        }
    }

    fn load(&self) -> Result<()> {
        match self {
            Self::KpatchCli { path } => {
                let mut cmd = Command::new(path);
                cmd.args(["kpm", "load", KPM_MODULE_FILE]);
                let out = cmd.output()?;
                if out.status.success() {
                    Ok(())
                } else {
                    Err(format!("kpm load failed with status {}", out.status).into())
                }
            }
            Self::ApatchSupercall { key, style } => {
                let path = CString::new(KPM_MODULE_FILE)?;
                let key = CString::new(key.as_str())?;
                let rc = unsafe {
                    syscall(
                        APATCH_SUPERCALL_NR,
                        key.as_ptr(),
                        supercall_cmd(*style, SUPERCALL_KPM_LOAD),
                        path.as_ptr(),
                        ptr::null::<c_char>(),
                        ptr::null_mut::<c_void>(),
                    )
                };
                supercall_ok(rc, "kpm load")
            }
        }
    }

    fn ctl0_config(&self, wire: &str) -> Result<()> {
        match self {
            Self::KpatchCli { path } => run_kpatch_kpm_ctl0_config(path, wire),
            Self::ApatchSupercall { key, style } => apatch_kpm_ctl0_config(key, *style, wire),
        }
    }

    fn ctl0_read(&self, wire: &str) -> Result<String> {
        match self {
            Self::KpatchCli { path } => run_kpatch_kpm_ctl0_read(path, wire),
            Self::ApatchSupercall { key, style } => apatch_kpm_ctl0_read(key, *style, wire),
        }
    }
}

fn kpatch_kpm_list_contains(kpatch: &Path) -> Result<bool> {
    let mut cmd = Command::new(kpatch);
    cmd.args(["kpm", "list"]);
    let out = cmd.output()?;
    if !out.status.success() {
        return Ok(false);
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.split_whitespace().any(|token| token == KPM_NAME))
}

fn kpatch_hello(kpatch: &Path) -> Result<()> {
    let mut cmd = Command::new(kpatch);
    cmd.arg("hello");
    let out = cmd.output()?;
    if out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty() {
        Ok(())
    } else {
        Err(format!(
            "KernelPatch inactive or kpatch hello failed with status {}",
            out.status
        )
        .into())
    }
}

fn run_kpatch_kpm_ctl0_config(kpatch: &Path, wire: &str) -> Result<()> {
    let mut cmd = Command::new(kpatch);
    cmd.args(["kpm", "ctl0", KPM_NAME, wire]);
    let out = cmd.output()?;
    if kpatch_ctl0_config_status_ok(out.status, wire) {
        Ok(())
    } else {
        Err(format!("kpm ctl0 failed with status {}", out.status).into())
    }
}

fn run_kpatch_kpm_ctl0_read(kpatch: &Path, wire: &str) -> Result<String> {
    let mut cmd = Command::new(kpatch);
    cmd.args(["kpm", "ctl0", KPM_NAME, wire]);
    let out = cmd.output()?;
    if out.status.success() {
        Ok(String::from_utf8(out.stdout)?)
    } else {
        Err(format!("kpm ctl0 read failed with status {}", out.status).into())
    }
}

fn kpatch_ctl0_config_status_ok(status: std::process::ExitStatus, wire: &str) -> bool {
    if status.success() {
        return true;
    }
    let Some(expected_targets) = parse_config(wire.as_bytes()).map(|cfg| cfg.targets.len()) else {
        return false;
    };
    // Compatibility with older vpnhide KPM builds: `ctl0 config` returned the
    // applied target count, which KPatch-Next exposes as the shell exit status.
    status.code() == Some(expected_targets as i32)
}

fn apatch_probe(key: &str) -> Result<ApatchCommandStyle> {
    let candidates = apatch_command_candidates();
    let mut failures = Vec::new();
    for style in candidates {
        let rc = apatch_hello(key, style)?;
        if rc == SUPERCALL_HELLO_MAGIC {
            return Ok(style);
        }
        failures.push(format!("{style:?}: rc={rc}"));
    }
    Err(format!(
        "KernelPatch inactive or bad APatch SuperKey (hello attempts: {})",
        failures.join(", ")
    )
    .into())
}

fn apatch_hello(key: &str, style: ApatchCommandStyle) -> Result<c_long> {
    let key = CString::new(key)?;
    let rc = unsafe {
        syscall(
            APATCH_SUPERCALL_NR,
            key.as_ptr(),
            supercall_cmd(style, SUPERCALL_HELLO),
        )
    };
    Ok(rc)
}

fn apatch_kpm_list(key: &str, style: ApatchCommandStyle) -> Result<String> {
    let key = CString::new(key)?;
    let mut buf = [0u8; 4096];
    let rc = unsafe {
        syscall(
            APATCH_SUPERCALL_NR,
            key.as_ptr(),
            supercall_cmd(style, SUPERCALL_KPM_LIST),
            buf.as_mut_ptr().cast::<c_char>(),
            buf.len() as c_long,
        )
    };
    supercall_ok(rc, "kpm list")?;
    let len = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    Ok(String::from_utf8_lossy(&buf[..len]).into_owned())
}

fn apatch_kpm_ctl0_config(key: &str, style: ApatchCommandStyle, wire: &str) -> Result<()> {
    let (rc, _) = apatch_kpm_ctl0_raw(key, style, wire)?;
    supercall_ok(rc, "kpm ctl0")
}

fn apatch_kpm_ctl0_read(key: &str, style: ApatchCommandStyle, wire: &str) -> Result<String> {
    let (rc, out) = apatch_kpm_ctl0_raw(key, style, wire)?;
    supercall_ok(rc, "kpm ctl0")?;
    let len = apatch_output_len(rc, &out);
    Ok(String::from_utf8_lossy(&out[..len]).into_owned())
}

fn apatch_kpm_ctl0_raw(
    key: &str,
    style: ApatchCommandStyle,
    wire: &str,
) -> Result<(c_long, [u8; 4096])> {
    let key = CString::new(key)?;
    let name = CString::new(KPM_NAME)?;
    let wire = CString::new(wire)?;
    let mut out = [0u8; 4096];
    let rc = unsafe {
        syscall(
            APATCH_SUPERCALL_NR,
            key.as_ptr(),
            supercall_cmd(style, SUPERCALL_KPM_CONTROL),
            name.as_ptr(),
            wire.as_ptr(),
            out.as_mut_ptr().cast::<c_char>(),
            out.len() as c_long,
        )
    };
    Ok((rc, out))
}

fn apatch_output_len(rc: c_long, out: &[u8]) -> usize {
    if rc > 0 {
        return usize::try_from(rc).unwrap_or(out.len()).min(out.len());
    }
    out.iter().position(|b| *b == 0).unwrap_or(0)
}

fn apatch_command_candidates() -> Vec<ApatchCommandStyle> {
    let mut styles = Vec::new();
    if let Some(version) = apatch_kernel_version_hint() {
        push_apatch_style(&mut styles, ApatchCommandStyle::Versioned(version));
    }
    push_apatch_style(
        &mut styles,
        ApatchCommandStyle::Versioned(APATCH_SUPERCALL_DEFAULT_VERSION_CODE),
    );
    for version in APATCH_SUPERCALL_VERSION_FALLBACKS {
        push_apatch_style(&mut styles, ApatchCommandStyle::Versioned(*version));
    }
    push_apatch_style(&mut styles, ApatchCommandStyle::Raw);
    styles
}

fn push_apatch_style(styles: &mut Vec<ApatchCommandStyle>, style: ApatchCommandStyle) {
    if !styles.contains(&style) {
        styles.push(style);
    }
}

fn apatch_kernel_version_hint() -> Option<c_long> {
    let out = Command::new("dmesg").output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_apatch_kernel_version_hint(&String::from_utf8_lossy(&out.stdout))
}

fn parse_apatch_kernel_version_hint(log: &str) -> Option<c_long> {
    const MARKER: &str = "KP KernelPatch Version:";
    for line in log.lines().rev() {
        let Some((_, tail)) = line.split_once(MARKER) else {
            continue;
        };
        let Some(value) = tail.split_whitespace().next() else {
            continue;
        };
        let value = value.trim_start_matches("0x");
        if value.is_empty() {
            continue;
        }
        if let Ok(version) = c_long::from_str_radix(value, 16) {
            return Some(version);
        }
    }
    None
}

fn supercall_cmd(style: ApatchCommandStyle, cmd: c_long) -> c_long {
    match style {
        ApatchCommandStyle::Versioned(version) => {
            (version << 32) | (APATCH_SUPERCALL_MAGIC << 16) | (cmd & 0xffff)
        }
        ApatchCommandStyle::Raw => cmd,
    }
}

fn supercall_ok(rc: c_long, op: &str) -> Result<()> {
    if rc >= 0 {
        Ok(())
    } else {
        Err(format!("{op} supercall failed with rc={rc}").into())
    }
}

fn build_ports_ruleset(
    chain: &str,
    loopback: &str,
    udp_reject: &str,
    targets: &BTreeMap<u32, PortUidPolicy>,
) -> String {
    let mut out = String::new();
    out.push_str("*filter\n");
    out.push_str(&format!(":{chain} - [0:0]\n"));
    for (uid, policy) in targets {
        if policy.all_ports {
            append_full_ports_rules(&mut out, chain, loopback, udp_reject, *uid);
        } else {
            for rule in &policy.rules {
                append_port_rule(&mut out, chain, loopback, udp_reject, *uid, *rule);
            }
        }
    }
    out.push_str(&format!("-A {chain} -j RETURN\n"));
    out.push_str("COMMIT\n");
    out
}

fn append_full_ports_rules(
    out: &mut String,
    chain: &str,
    loopback: &str,
    udp_reject: &str,
    uid: u32,
) {
    out.push_str(&format!(
        "-A {chain} -m owner --uid-owner {uid} -d {loopback} -p tcp -j REJECT --reject-with tcp-reset\n",
    ));
    out.push_str(&format!(
        "-A {chain} -m owner --uid-owner {uid} -d {loopback} -p udp -j REJECT --reject-with {udp_reject}\n",
    ));
}

fn append_port_rule(
    out: &mut String,
    chain: &str,
    loopback: &str,
    udp_reject: &str,
    uid: u32,
    rule: PortRule,
) {
    match rule.protocol {
        PortProtocol::Both => {
            append_protocol_port_rule(out, chain, loopback, "tcp", "tcp-reset", uid, rule);
            append_protocol_port_rule(out, chain, loopback, "udp", udp_reject, uid, rule);
        }
        PortProtocol::Tcp => {
            append_protocol_port_rule(out, chain, loopback, "tcp", "tcp-reset", uid, rule);
        }
        PortProtocol::Udp => {
            append_protocol_port_rule(out, chain, loopback, "udp", udp_reject, uid, rule);
        }
    }
}

fn append_protocol_port_rule(
    out: &mut String,
    chain: &str,
    loopback: &str,
    proto: &str,
    reject: &str,
    uid: u32,
    rule: PortRule,
) {
    out.push_str(&format!(
        "-A {chain} -m owner --uid-owner {uid} -d {loopback} -p {proto} --dport {} -j REJECT --reject-with {reject}\n",
        port_match(rule),
    ));
}

fn port_match(rule: PortRule) -> String {
    let end = rule.end_port();
    if rule.start == end {
        rule.start.to_string()
    } else {
        format!("{}:{end}", rule.start)
    }
}

fn apply_ports_rules(rules: &PortsRuleset) -> Result<()> {
    let _ = Command::new("iptables").args(["-N", PORTS_CHAIN4]).status();
    let _ = Command::new("ip6tables")
        .args(["-N", PORTS_CHAIN6])
        .status();

    let rc4 = run_with_stdin("iptables-restore", &["--noflush"], &rules.ipv4)?;
    let rc6 = run_with_stdin("ip6tables-restore", &["--noflush"], &rules.ipv6)?;

    ensure_output_jump("iptables", PORTS_CHAIN4)?;
    ensure_output_jump("ip6tables", PORTS_CHAIN6)?;

    if !rc4.success() || !rc6.success() {
        return Err(format!("ports apply failed: rc4={rc4} rc6={rc6}").into());
    }
    Ok(())
}

fn run_with_stdin(program: &str, args: &[&str], stdin: &str) -> Result<std::process::ExitStatus> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()?;
    {
        let pipe = child.stdin.as_mut().ok_or("stdin pipe unavailable")?;
        pipe.write_all(stdin.as_bytes())?;
    }
    Ok(child.wait()?)
}

fn ensure_output_jump(program: &str, chain: &str) -> Result<()> {
    let check = Command::new(program)
        .args(["-C", "OUTPUT", "-j", chain])
        .status()?;
    if check.success() {
        return Ok(());
    }
    let insert = Command::new(program)
        .args(["-I", "OUTPUT", "-j", chain])
        .status()?;
    if insert.success() {
        Ok(())
    } else {
        Err(format!("{program} failed to insert OUTPUT jump to {chain}: {insert}").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn parses_pm_package_uids_for_all_profiles() {
        let map = parse_pm_packages(
            "package:com.example.one uid:10123,1010123\n\
             package:com.example.two uid:10234\n\
             package:bad.without.uid\n",
        );
        assert_eq!(map.uids_for("com.example.one"), &[10123, 1010123]);
        assert_eq!(map.uids_for("com.example.two"), &[10234]);
        assert!(map.uids_for("bad.without.uid").is_empty());
    }

    #[test]
    fn projects_native_roles_to_wire() {
        let cfg = parse_canonical(
            r#"{
              "version": 1,
              "debug": true,
              "apps": {
                "com.example.disabled": { "native": false },
                "com.example.full": { "native": true },
                "com.example.partial": { "native": ["sock_ioctl"] }
              }
            }"#,
        )
        .unwrap();
        let resolver = parse_pm_packages(
            "package:com.example.full uid:10123,1010123\n\
             package:com.example.partial uid:10234\n\
             package:com.example.disabled uid:10345\n",
        );
        assert_eq!(
            project_native_with_resolver(&cfg, &resolver),
            "vpnhide 1 config\n\
             debug 1\n\
             target 0x278b 0x3ff\n\
             target 0x27fa 0x40\n\
             target 0xf69cb 0x3ff\n",
        );
    }

    #[test]
    fn native_projection_ignores_non_kernel_hook_names() {
        let cfg = parse_canonical(
            r#"{
              "version": 1,
              "debug": false,
              "apps": {
                "com.example.java": { "native": ["lsposed_network"] }
              }
            }"#,
        )
        .unwrap();
        let resolver = parse_pm_packages("package:com.example.java uid:10123\n");

        assert_eq!(
            project_native_with_resolver(&cfg, &resolver),
            "vpnhide 1 config\ndebug 0\n",
        );
    }

    #[test]
    fn projects_backend_specific_native_hook_overrides() {
        let cfg = parse_canonical(
            r#"{
              "version": 1,
              "apps": {
                "com.example.app": {
                  "native": {
                    "enabled": true,
                    "kernel": ["sock_ioctl"],
                    "zygisk": ["zygisk_ioctl", "zygisk_recvfrom_chk"]
                  }
                }
              }
            }"#,
        )
        .unwrap();
        let resolver = parse_pm_packages("package:com.example.app uid:10234\n");

        assert_eq!(
            project_native_with_resolver(&cfg, &resolver),
            "vpnhide 1 config\n\
             debug 0\n\
             target 0x27fa 0x40\n",
        );
        assert_eq!(
            project_native_with_resolver_for_family(&cfg, &resolver, NativeHookFamily::Zygisk),
            "vpnhide 1 config\n\
             debug 0\n\
             target 0x27fa 0x1040000\n",
        );
    }

    #[test]
    fn legacy_native_hook_list_is_kernel_only_and_zygisk_defaults_to_all() {
        let cfg = parse_canonical(
            r#"{
              "version": 1,
              "apps": {
                "com.example.app": { "native": ["sock_ioctl"] }
              }
            }"#,
        )
        .unwrap();
        let resolver = parse_pm_packages("package:com.example.app uid:10234\n");

        assert_eq!(
            project_native_with_resolver(&cfg, &resolver),
            "vpnhide 1 config\n\
             debug 0\n\
             target 0x27fa 0x40\n",
        );
        assert_eq!(
            project_native_with_resolver_for_family(&cfg, &resolver, NativeHookFamily::Zygisk),
            "vpnhide 1 config\n\
             debug 0\n\
             target 0x27fa 0x1fc0000\n",
        );
    }

    #[test]
    fn empty_legacy_native_hook_list_is_disabled_for_every_backend() {
        let cfg = parse_canonical(
            r#"{
              "version": 1,
              "apps": {
                "com.example.app": { "native": [] }
              }
            }"#,
        )
        .unwrap();
        let resolver = parse_pm_packages("package:com.example.app uid:10234\n");

        assert_eq!(
            project_native_with_resolver(&cfg, &resolver),
            "vpnhide 1 config\ndebug 0\n",
        );
        assert_eq!(
            project_native_with_resolver_for_family(&cfg, &resolver, NativeHookFamily::Zygisk),
            "vpnhide 1 config\ndebug 0\n",
        );
    }

    #[test]
    fn parses_shared_storage_fixture() {
        let cfg =
            parse_canonical(include_str!("../../../testdata/storage_config_v1.json")).unwrap();

        assert!(cfg.debug);
        assert!(cfg.settings.remember_superkey);
        assert_eq!(
            cfg.apps.get("com.example.bank").unwrap().native,
            NativeSelection::Enabled(true),
        );
        let proxy = cfg.apps.get("org.example.proxy").unwrap();
        assert_eq!(
            proxy.native,
            NativeSelection::Hooks(vec![
                "fib_route_seq_show".to_owned(),
                "sock_ioctl".to_owned()
            ]),
        );
        // Per-hook Java selection in the fixture: the array form must parse and
        // collapse to "java enabled" without breaking the native config read.
        assert!(proxy.java);
    }

    #[test]
    fn parses_per_hook_java_selection_without_breaking_native() {
        // The canonical the app writes when a user picks individual Java hooks:
        // "java" is a string array, not a bool. A bool-only field used to make
        // serde reject the whole config, silently disabling every native target.
        let cfg = parse_canonical(
            r#"{
              "version": 1,
              "apps": {
                "com.example.partialjava": {
                  "java": ["lsposed_network", "lsposed_network_info"],
                  "native": true
                },
                "com.example.emptyjava": { "java": [], "native": true }
              }
            }"#,
        )
        .unwrap();
        assert!(cfg.apps.get("com.example.partialjava").unwrap().java);
        // An empty array means no Java hooks -> role disabled.
        assert!(!cfg.apps.get("com.example.emptyjava").unwrap().java);

        let resolver = parse_pm_packages(
            "package:com.example.partialjava uid:10123\n\
             package:com.example.emptyjava uid:10124\n",
        );
        // Native projection is unaffected: both apps still get the kernel mask.
        assert_eq!(
            project_native_with_resolver(&cfg, &resolver),
            "vpnhide 1 config\n\
             debug 0\n\
             target 0x278b 0x3ff\n\
             target 0x278c 0x3ff\n",
        );
    }

    #[test]
    fn projects_ports_roles_to_iptables_rulesets() {
        let cfg = parse_canonical(
            r#"{
              "version": 1,
              "apps": {
                "com.example.disabled": { "ports": false },
                "com.example.ports": { "ports": true },
                "com.example.system": { "ports": true }
              }
            }"#,
        )
        .unwrap();
        let resolver = parse_pm_packages(
            "package:com.example.ports uid:10123,1010123\n\
             package:com.example.system uid:999\n\
             package:com.example.disabled uid:10345\n",
        );

        let rules = project_ports_with_resolver(&cfg, &resolver);

        assert_eq!(rules.target_count, 2);
        assert_eq!(
            rules.ipv4,
            "*filter\n\
             :vpnhide_out - [0:0]\n\
             -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p tcp -j REJECT --reject-with tcp-reset\n\
             -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p udp -j REJECT --reject-with icmp-port-unreachable\n\
             -A vpnhide_out -m owner --uid-owner 1010123 -d 127.0.0.0/8 -p tcp -j REJECT --reject-with tcp-reset\n\
             -A vpnhide_out -m owner --uid-owner 1010123 -d 127.0.0.0/8 -p udp -j REJECT --reject-with icmp-port-unreachable\n\
             -A vpnhide_out -j RETURN\n\
             COMMIT\n",
        );
        assert_eq!(
            rules.ipv6,
            "*filter\n\
             :vpnhide_out6 - [0:0]\n\
             -A vpnhide_out6 -m owner --uid-owner 10123 -d ::1 -p tcp -j REJECT --reject-with tcp-reset\n\
             -A vpnhide_out6 -m owner --uid-owner 10123 -d ::1 -p udp -j REJECT --reject-with icmp6-port-unreachable\n\
             -A vpnhide_out6 -m owner --uid-owner 1010123 -d ::1 -p tcp -j REJECT --reject-with tcp-reset\n\
             -A vpnhide_out6 -m owner --uid-owner 1010123 -d ::1 -p udp -j REJECT --reject-with icmp6-port-unreachable\n\
             -A vpnhide_out6 -j RETURN\n\
             COMMIT\n",
        );
    }

    #[test]
    fn projects_custom_ports_policy_to_dport_rules() {
        let cfg = parse_canonical(
            r#"{
              "version": 1,
              "apps": {
                "com.example.proxy": {
                  "ports": true,
                  "portPolicy": {
                    "mode": "custom",
                    "rules": [
                      { "protocol": "tcp", "start": 7890, "end": 7892 },
                      { "protocol": "udp", "start": 5353 },
                      { "start": 1080 }
                    ]
                  }
                }
              }
            }"#,
        )
        .unwrap();
        let resolver = parse_pm_packages("package:com.example.proxy uid:10123\n");

        let rules = project_ports_with_resolver(&cfg, &resolver);

        assert_eq!(rules.target_count, 1);
        assert_eq!(
            rules.ipv4,
            "*filter\n\
             :vpnhide_out - [0:0]\n\
             -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p tcp --dport 1080 -j REJECT --reject-with tcp-reset\n\
             -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p udp --dport 1080 -j REJECT --reject-with icmp-port-unreachable\n\
             -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p udp --dport 5353 -j REJECT --reject-with icmp-port-unreachable\n\
             -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p tcp --dport 7890:7892 -j REJECT --reject-with tcp-reset\n\
             -A vpnhide_out -j RETURN\n\
             COMMIT\n",
        );
    }

    #[test]
    fn shared_uid_full_ports_policy_wins_over_ranges() {
        let cfg = parse_canonical(
            r#"{
              "version": 1,
              "apps": {
                "com.example.full": { "ports": true },
                "com.example.range": {
                  "ports": true,
                  "portPolicy": {
                    "mode": "custom",
                    "rules": [{ "protocol": "tcp", "start": 7890 }]
                  }
                }
              }
            }"#,
        )
        .unwrap();
        let resolver = parse_pm_packages(
            "package:com.example.full uid:10123\n\
             package:com.example.range uid:10123\n",
        );

        let rules = project_ports_with_resolver(&cfg, &resolver);

        assert_eq!(rules.target_count, 1);
        assert!(
            rules
                .ipv4
                .contains("-p tcp -j REJECT --reject-with tcp-reset")
        );
        assert!(!rules.ipv4.contains("--dport"));
    }

    #[test]
    fn rejects_invalid_ports_policy_ranges() {
        assert!(
            parse_canonical(
                r#"{
                  "version": 1,
                  "apps": {
                    "com.example.bad": {
                      "ports": true,
                      "portPolicy": {
                        "mode": "custom",
                        "rules": [{ "start": 0 }]
                      }
                    }
                  }
                }"#,
            )
            .is_err(),
        );
        assert!(
            parse_canonical(
                r#"{
                  "version": 1,
                  "apps": {
                    "com.example.bad": {
                      "ports": true,
                      "portPolicy": {
                        "mode": "custom",
                        "rules": [{ "start": 9000, "end": 8000 }]
                      }
                    }
                  }
                }"#,
            )
            .is_err(),
        );
    }

    #[test]
    fn projects_shared_fixture_ports_role() {
        let cfg =
            parse_canonical(include_str!("../../../testdata/storage_config_v1.json")).unwrap();
        let resolver = parse_pm_packages(
            "package:org.example.proxy uid:10177\n\
             package:com.example.bank uid:10178\n",
        );

        let rules = project_ports_with_resolver(&cfg, &resolver);

        assert_eq!(rules.target_count, 1);
        assert!(rules.ipv4.contains("--uid-owner 10177"));
        assert!(!rules.ipv4.contains("--uid-owner 10178"));
    }

    #[test]
    fn absent_canonical_projects_to_empty_config_without_pm() {
        assert_eq!(
            project_native(empty_canonical_json()).unwrap(),
            "vpnhide 1 config\ndebug 0\n",
        );
    }

    #[test]
    fn pm_ready_check_matches_literal_package_token() {
        assert!(pm_output_has_package(
            "package:dev.okhsunrog.vpnhide uid:10123\n",
            APP_PACKAGE,
        ));
        assert!(!pm_output_has_package(
            "package:dev.okhsunrog.vpnhide.extra uid:10123\n",
            APP_PACKAGE,
        ));
    }

    #[test]
    fn apatch_supercall_command_keeps_kpm_command_in_low_bits() {
        assert_eq!(
            supercall_cmd(
                ApatchCommandStyle::Versioned(APATCH_SUPERCALL_DEFAULT_VERSION_CODE),
                SUPERCALL_KPM_CONTROL,
            ),
            (APATCH_SUPERCALL_DEFAULT_VERSION_CODE << 32)
                | (APATCH_SUPERCALL_MAGIC << 16)
                | SUPERCALL_KPM_CONTROL,
        );
        assert_eq!(
            supercall_cmd(
                ApatchCommandStyle::Versioned(0x000c02),
                SUPERCALL_KPM_CONTROL
            ) & 0xffff,
            0x1022,
        );
        assert_eq!(
            supercall_cmd(ApatchCommandStyle::Raw, SUPERCALL_KPM_CONTROL),
            0x1022
        );
        assert_eq!(
            supercall_cmd(ApatchCommandStyle::Versioned(0x000c02), SUPERCALL_HELLO) & 0xffff,
            0x1000,
        );
        assert_eq!(SUPERCALL_HELLO_MAGIC, 0x11581158);
    }

    #[test]
    fn apatch_kernel_version_hint_parses_dmesg() {
        let log = "\
[    0.000000] KP KernelPatch Version: c02
[    0.000000] KP KernelPatch Config: 2
";
        assert_eq!(parse_apatch_kernel_version_hint(log), Some(0xc02));
    }

    #[test]
    fn projection_is_bounded_to_backend_target_capacity() {
        let apps = (0..70)
            .map(|i| {
                (
                    format!("com.example.{i:02}"),
                    AppConfig {
                        native: NativeSelection::Enabled(true),
                        ..AppConfig::default()
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let cfg = CanonicalConfig {
            version: 1,
            debug: false,
            apps,
            settings: Settings::default(),
        };
        let pm = (0..70)
            .map(|i| format!("package:com.example.{i:02} uid:{}", 10_000 + i))
            .collect::<Vec<_>>()
            .join("\n");
        let wire = project_native_with_resolver(&cfg, &parse_pm_packages(&pm));

        assert_eq!(
            wire.lines()
                .filter(|line| line.starts_with("target "))
                .count(),
            64
        );
    }

    #[test]
    fn kpatch_ctl0_accepts_config_target_count_exit_codes() {
        let one_target = "vpnhide 1 config\ndebug 0\ntarget 0x123 0x1\n";
        assert!(kpatch_ctl0_config_status_ok(
            std::process::ExitStatus::from_raw(0),
            "vpnhide 1 config\ndebug 0\n"
        ));
        assert!(kpatch_ctl0_config_status_ok(
            std::process::ExitStatus::from_raw(1 << 8),
            one_target
        ));
        assert!(!kpatch_ctl0_config_status_ok(
            std::process::ExitStatus::from_raw(2 << 8),
            one_target
        ));
        assert!(!kpatch_ctl0_config_status_ok(
            std::process::ExitStatus::from_raw(1 << 8),
            "not vpnhide config\n"
        ));
        assert!(!kpatch_ctl0_config_status_ok(
            std::process::ExitStatus::from_raw(255 << 8),
            one_target
        ));
        assert!(!kpatch_ctl0_config_status_ok(
            std::process::ExitStatus::from_raw(15),
            one_target
        ));
    }
}
