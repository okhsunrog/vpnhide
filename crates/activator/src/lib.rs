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
use vpnhide_protocol::hook_ids::{HOOK_NAMES, KERNEL_HOOK_MASK};
use vpnhide_protocol::{format_config, parse_config};

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
const MAX_NATIVE_TARGETS: usize = 64;
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
    #[serde(default)]
    pub java: bool,
    #[serde(default)]
    pub native: NativeSelection,
    #[serde(default)]
    pub app_hiding: bool,
    #[serde(default)]
    pub ports: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum NativeSelection {
    Enabled(bool),
    Hooks(Vec<String>),
}

impl Default for NativeSelection {
    fn default() -> Self {
        Self::Enabled(false)
    }
}

impl NativeSelection {
    fn hookmask(&self) -> Option<u32> {
        match self {
            NativeSelection::Enabled(false) => None,
            NativeSelection::Enabled(true) => Some(KERNEL_HOOK_MASK),
            NativeSelection::Hooks(names) => {
                let mask =
                    names.iter().fold(0u32, |acc, name| acc | hook_bit(name)) & KERNEL_HOOK_MASK;
                (mask != 0).then_some(mask)
            }
        }
    }
}

fn schema_version() -> u32 {
    1
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
    Ok(cfg)
}

pub fn project_native(json: &str) -> Result<String> {
    project_native_with_pm_wait(json, PmReadyWait::Bounded(PM_READY_ATTEMPTS))
}

fn project_native_with_pm_wait(json: &str, wait: PmReadyWait) -> Result<String> {
    let cfg = parse_canonical(json)?;
    if !has_native_targets(&cfg) {
        return Ok(format_config(cfg.debug, &[]));
    }
    let resolver = PackageUidMap::from_pm_with_wait(wait)?;
    Ok(project_native_with_resolver(&cfg, &resolver))
}

pub fn project_native_with_resolver(cfg: &CanonicalConfig, resolver: &PackageUidMap) -> String {
    let mut by_uid = BTreeMap::<u32, u32>::new();
    for (pkg, app) in &cfg.apps {
        let Some(mask) = app.native.hookmask() else {
            continue;
        };
        for uid in resolver.uids_for(pkg) {
            by_uid
                .entry(*uid)
                .and_modify(|existing| *existing |= mask)
                .or_insert(mask);
        }
    }
    let targets = by_uid
        .into_iter()
        .take(MAX_NATIVE_TARGETS)
        .map(|(uid, hookmask)| Target { uid, hookmask })
        .collect::<Vec<_>>();
    format_config(cfg.debug, &targets)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortsRuleset {
    pub ipv4: String,
    pub ipv6: String,
    pub target_count: usize,
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
    let mut uids = BTreeSet::<u32>::new();
    for (pkg, app) in &cfg.apps {
        if !app.ports {
            continue;
        }
        for uid in resolver.uids_for(pkg) {
            if *uid >= 10_000 {
                uids.insert(*uid);
            }
        }
    }
    PortsRuleset {
        ipv4: build_ports_ruleset(PORTS_CHAIN4, "127.0.0.1", "icmp-port-unreachable", &uids),
        ipv6: build_ports_ruleset(PORTS_CHAIN6, "::1", "icmp6-port-unreachable", &uids),
        target_count: uids.len(),
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
    let wire = project_native_with_pm_wait(&read_canonical()?, wait)?;
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
    let wire = project_native_with_pm_wait(&read_canonical()?, wait)?;
    write_atomic(Path::new(ZYGISK_RUNTIME_CONFIG), wire.as_bytes(), 0o644)
}

pub fn activate_kpm() -> Result<()> {
    activate_kpm_with_pm_wait(PmReadyWait::Bounded(PM_READY_ATTEMPTS), true)
}

pub fn activate_kpm_boot() -> Result<()> {
    activate_kpm_with_pm_wait(PmReadyWait::Forever, false)
}

fn activate_kpm_with_pm_wait(wait: PmReadyWait, conflict_is_error: bool) -> Result<()> {
    if skip_kpm_for_kmod_conflict(conflict_is_error)? {
        return Ok(());
    }
    let wire = project_native_with_pm_wait(&read_canonical()?, wait)?;
    if skip_kpm_for_kmod_conflict(conflict_is_error)? {
        return Ok(());
    }
    let client = KpmClient::detect()?;
    client.ensure_loaded()?;
    client.ctl0(&wire)
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

fn has_native_targets(cfg: &CanonicalConfig) -> bool {
    cfg.apps.values().any(|app| app.native.hookmask().is_some())
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

fn kmod_backend_present() -> bool {
    Path::new(KMOD_CTL).exists()
        || (Path::new(KMOD_MODULE_DIR).is_dir()
            && !Path::new(KMOD_MODULE_DIR).join("disable").exists())
}

fn skip_kpm_for_kmod_conflict(conflict_is_error: bool) -> Result<bool> {
    if !kmod_backend_present() {
        return Ok(false);
    }
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

fn find_kpatch() -> Option<PathBuf> {
    [
        "kpatch",
        "/data/adb/ksu/bin/kpatch",
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

    fn ctl0(&self, wire: &str) -> Result<()> {
        match self {
            Self::KpatchCli { path } => run_kpatch_kpm_ctl0(path, wire),
            Self::ApatchSupercall { key, style } => apatch_kpm_ctl0(key, *style, wire),
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

fn run_kpatch_kpm_ctl0(kpatch: &Path, wire: &str) -> Result<()> {
    let mut cmd = Command::new(kpatch);
    cmd.args(["kpm", "ctl0", KPM_NAME, wire]);
    let out = cmd.output()?;
    if kpatch_ctl0_config_status_ok(out.status, wire) {
        Ok(())
    } else {
        Err(format!("kpm ctl0 failed with status {}", out.status).into())
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

fn apatch_kpm_ctl0(key: &str, style: ApatchCommandStyle, wire: &str) -> Result<()> {
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
    supercall_ok(rc, "kpm ctl0")
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
    uids: &BTreeSet<u32>,
) -> String {
    let mut out = String::new();
    out.push_str("*filter\n");
    out.push_str(&format!(":{chain} - [0:0]\n"));
    for uid in uids {
        out.push_str(&format!(
            "-A {chain} -m owner --uid-owner {uid} -d {loopback} -p tcp -j REJECT --reject-with tcp-reset\n",
        ));
        out.push_str(&format!(
            "-A {chain} -m owner --uid-owner {uid} -d {loopback} -p udp -j REJECT --reject-with {udp_reject}\n",
        ));
    }
    out.push_str(&format!("-A {chain} -j RETURN\n"));
    out.push_str("COMMIT\n");
    out
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
    fn parses_shared_storage_fixture() {
        let cfg =
            parse_canonical(include_str!("../../../testdata/storage_config_v1.json")).unwrap();

        assert!(cfg.debug);
        assert!(cfg.settings.remember_superkey);
        assert_eq!(
            cfg.apps.get("com.example.bank").unwrap().native,
            NativeSelection::Enabled(true),
        );
        assert_eq!(
            cfg.apps.get("org.example.proxy").unwrap().native,
            NativeSelection::Hooks(vec![
                "fib_route_seq_show".to_owned(),
                "sock_ioctl".to_owned()
            ]),
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
             -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.1 -p tcp -j REJECT --reject-with tcp-reset\n\
             -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.1 -p udp -j REJECT --reject-with icmp-port-unreachable\n\
             -A vpnhide_out -m owner --uid-owner 1010123 -d 127.0.0.1 -p tcp -j REJECT --reject-with tcp-reset\n\
             -A vpnhide_out -m owner --uid-owner 1010123 -d 127.0.0.1 -p udp -j REJECT --reject-with icmp-port-unreachable\n\
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
