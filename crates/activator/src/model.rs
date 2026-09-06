use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use vpnhide_protocol::Target;
use vpnhide_protocol::format_config;
use vpnhide_protocol::hook_ids::{
    Hook, BUILTIN_HOOK_MASK, KERNEL_HOOK_MASK, KMOD_HOOK_MASK, KPM_HOOK_MASK, ZYGISK_HOOK_MASK,
};

use crate::ports::build_ports_ruleset;
use crate::{
    APP_PACKAGE, MAX_NATIVE_TARGETS, NO_DEFAULT_MASK, PER_USER_RANGE, PM_READY_ATTEMPTS,
    PORTS_CHAIN4, PORTS_CHAIN6, PmReadyWait, Result, has_native_targets, has_ports_targets,
    is_app_uid, pm_list_packages, pm_list_users, wait_for_pm_ready,
};

pub const OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS: &str = Hook::FilesystemIfacePaths.name();

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
    #[serde(default)]
    pub optional_features: BTreeSet<String>,
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

#[derive(Debug)]
pub struct PortsActivationReport {
    pub target_count: usize,
    pub log: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Both,
    Tcp,
    Udp,
}

impl PortRule {
    pub(crate) fn end_port(self) -> u16 {
        self.end.unwrap_or(self.start)
    }

    pub(crate) fn normalized(self) -> Self {
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
pub(crate) enum NativeHookFamily {
    Kmod,
    Kpm,
    Builtin,
    Zygisk,
}

impl NativeHookFamily {
    fn full_set(self) -> HookSet {
        match self {
            NativeHookFamily::Kmod => HookSet::from_bits(KERNEL_HOOK_MASK | KMOD_HOOK_MASK),
            NativeHookFamily::Kpm => HookSet::from_bits(KERNEL_HOOK_MASK | KPM_HOOK_MASK),
            NativeHookFamily::Builtin => {
                HookSet::from_bits(KERNEL_HOOK_MASK | BUILTIN_HOOK_MASK)
            }
            NativeHookFamily::Zygisk => HookSet::from_bits(ZYGISK_HOOK_MASK),
        }
    }

    /// True for backends with no separate loader ABI (no insmod module param,
    /// no ensure_loaded): the optional filesystem hook must be gated in the
    /// projected mask itself, because there is no load-time gate behind it.
    /// The .ko (module_param) and KPM (ensure_loaded) gate at load, so their
    /// mask may carry the bit unconditionally; the compiled-in built-in driver
    /// and zygisk cannot, so their mask is the only gate.
    fn filesystem_gated_in_mask(self) -> bool {
        matches!(self, NativeHookFamily::Builtin | NativeHookFamily::Zygisk)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HookSet(u32);

impl HookSet {
    const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    fn from_names(names: &[String]) -> Self {
        names
            .iter()
            .filter_map(|name| Hook::from_name(name))
            .fold(Self::default(), |hooks, hook| hooks.with(hook))
    }

    const fn with(self, hook: Hook) -> Self {
        Self(self.0 | hook.bit())
    }

    const fn without(self, hook: Hook) -> Self {
        Self(self.0 & !hook.bit())
    }

    const fn restricted_to(self, owner: Self) -> Self {
        Self(self.0 & owner.0)
    }

    fn merge(&mut self, other: Self) {
        self.0 |= other.0;
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn bits(self) -> u32 {
        self.0
    }
}

impl NativeSelection {
    pub(crate) fn hooks(&self, family: NativeHookFamily) -> Option<HookSet> {
        match self {
            NativeSelection::Enabled(false) => None,
            NativeSelection::Enabled(true) => Some(family.full_set()),
            NativeSelection::Hooks(names) => {
                if names.is_empty() {
                    return None;
                }
                let hooks = match family {
                    NativeHookFamily::Kmod
                    | NativeHookFamily::Kpm
                    | NativeHookFamily::Builtin => {
                        HookSet::from_names(names).restricted_to(family.full_set())
                    }
                    NativeHookFamily::Zygisk => family.full_set(),
                };
                (!hooks.is_empty()).then_some(hooks)
            }
            NativeSelection::Detailed(detail) => {
                if !detail.enabled {
                    return None;
                }
                let selected = match family {
                    NativeHookFamily::Kmod
                    | NativeHookFamily::Kpm
                    | NativeHookFamily::Builtin => &detail.kernel,
                    NativeHookFamily::Zygisk => &detail.zygisk,
                };
                let Some(names) = selected else {
                    return Some(family.full_set());
                };
                let hooks = HookSet::from_names(names).restricted_to(family.full_set());
                (!hooks.is_empty()).then_some(hooks)
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
        let users_output = pm_list_users()?;
        let user_ids = parse_pm_user_ids(&users_output);
        if user_ids.is_empty() {
            return Err("PackageManager returned no Android users".into());
        }

        let mut packages = BTreeMap::<String, Vec<u32>>::new();
        for user_id in user_ids {
            let user = user_id.to_string();
            let stdout = pm_list_packages(&["list", "packages", "-U", "--user", user.as_str()])?;
            merge_pm_packages(&mut packages, parse_pm_packages(&stdout));
        }
        Ok(Self { packages })
    }

    pub(crate) fn uids_for(&self, package: &str) -> &[u32] {
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
        let (Some(pkg_token), Some(uid_csv)) = (pkg, uid_csv) else {
            continue;
        };
        let pkg = pkg_token
            .rsplit_once('=')
            .map_or(pkg_token, |(_, package)| package);
        let uids = uid_csv
            .split(',')
            .filter_map(|s| s.parse::<u32>().ok())
            .collect::<Vec<_>>();
        if !uids.is_empty() {
            let existing = packages.entry(pkg.to_owned()).or_default();
            existing.extend(uids);
            existing.sort_unstable();
            existing.dedup();
        }
    }
    PackageUidMap { packages }
}

pub fn parse_pm_user_ids(output: &str) -> Vec<u32> {
    let mut users = output
        .lines()
        .filter_map(|line| {
            let after_prefix = line.split_once("UserInfo{")?.1;
            after_prefix.split_once(':')?.0.trim().parse::<u32>().ok()
        })
        .collect::<Vec<_>>();
    users.sort_unstable();
    users.dedup();
    users
}

fn merge_pm_packages(destination: &mut BTreeMap<String, Vec<u32>>, source: PackageUidMap) {
    for (package, uids) in source.packages {
        let existing = destination.entry(package).or_default();
        existing.extend(uids);
        existing.sort_unstable();
        existing.dedup();
    }
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
        NativeHookFamily::Kpm,
        PmReadyWait::Bounded(PM_READY_ATTEMPTS),
    )
}

pub(crate) fn project_native_with_pm_wait(
    json: &str,
    family: NativeHookFamily,
    wait: PmReadyWait,
) -> Result<String> {
    let cfg = parse_canonical(json)?;
    if !has_native_targets(&cfg, family) {
        return Ok(format_config(cfg.debug, NO_DEFAULT_MASK, &[]));
    }
    let resolver = PackageUidMap::from_pm_with_wait(wait)?;
    Ok(project_native_with_resolver_for_family(
        &cfg, &resolver, family,
    ))
}

pub(crate) fn project_native_with_resolver_for_family(
    cfg: &CanonicalConfig,
    resolver: &PackageUidMap,
    family: NativeHookFamily,
) -> String {
    let filesystem_enabled = cfg
        .settings
        .optional_features
        .contains(OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS);
    let mut by_uid = BTreeMap::<u32, HookSet>::new();
    for (pkg, app) in &cfg.apps {
        let Some(mut hooks) = app.native.hooks(family) else {
            continue;
        };
        // The .ko (module_param) and KPM (ensure_loaded) gate the optional
        // filesystem hook at load time, so their projected mask may carry the
        // bit unconditionally; runtime status reports the installed capability.
        // Backends with no separate loader ABI — zygisk (per-process mask) and
        // the compiled-in built-in driver (CONFIG_VPNHIDE_FS_HIDING=y, gated
        // only by the runtime mask) — have no load-time gate, so the toggle
        // must be projected into the mask itself, else it can never turn off.
        if family.filesystem_gated_in_mask() && !filesystem_enabled {
            hooks = hooks.without(Hook::FilesystemIfacePaths);
            if hooks.is_empty() {
                continue;
            }
        }
        // The APK is intentionally single-owner: only its main-profile copy
        // may manage the shared config. Keep its mandatory self target just as
        // singular here. An accidentally installed work/clone-profile copy is
        // blocked at startup and must not consume another backend slot.
        if pkg == APP_PACKAGE {
            if let Some(uid) = resolver
                .uids_for(pkg)
                .iter()
                .copied()
                .find(|uid| *uid < PER_USER_RANGE && is_app_uid(*uid))
            {
                by_uid
                    .entry(uid)
                    .and_modify(|existing| existing.merge(hooks))
                    .or_insert(hooks);
            }
            continue;
        }
        for uid in resolver.uids_for(pkg) {
            // Below the app range a uid is not an app but a platform identity
            // shared by many components — a package declaring sharedUserId
            // "android.uid.system" resolves to 1000, the same uid as
            // system_server. Since UID is the targeting key, listing one would
            // mean "hide from everything running as 1000", which is how a
            // device ends up believing it has no route. This is NOT the same
            // set as FLAG_SYSTEM: vendor-preinstalled apps keep ordinary 10xxx
            // uids and stay targetable. `project_ports_with_resolver` has
            // filtered the same way from the start; the native path had not.
            // Both kernel backends enforce it too, so this is the polite half.
            if !is_app_uid(*uid) {
                continue;
            }
            by_uid
                .entry(*uid)
                .and_modify(|existing| existing.merge(hooks))
                .or_insert(hooks);
        }
    }
    // `by_uid` is a BTreeMap, so iteration is ascending by UID; truncating would
    // silently drop the highest-UID (typically most-recently-installed) apps with
    // no diagnostic. Warn so a user with more native targets than the backend can
    // hold learns their protection is partial, instead of failing closed silently.
    if by_uid.len() > MAX_NATIVE_TARGETS {
        eprintln!("{}", native_target_capacity_warning(by_uid.len()));
    }
    let targets = by_uid
        .into_iter()
        .take(MAX_NATIVE_TARGETS)
        .map(|(uid, hooks)| Target {
            uid,
            hookmask: hooks.bits(),
        })
        .collect::<Vec<_>>();
    format_config(cfg.debug, NO_DEFAULT_MASK, &targets)
}

pub(crate) fn native_target_capacity_warning(total: usize) -> String {
    format!(
        "vpnhide-warning native_target_cap total={total} cap={MAX_NATIVE_TARGETS} dropped={}",
        total.saturating_sub(MAX_NATIVE_TARGETS),
    )
}

pub fn project_native_with_resolver(cfg: &CanonicalConfig, resolver: &PackageUidMap) -> String {
    project_native_with_resolver_for_family(cfg, resolver, NativeHookFamily::Kpm)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortsRuleset {
    pub ipv4: String,
    pub ipv6: String,
    pub target_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PortUidPolicy {
    pub(crate) all_ports: bool,
    pub(crate) rules: BTreeSet<PortRule>,
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

pub(crate) fn project_ports_with_pm_wait(json: &str, wait: PmReadyWait) -> Result<PortsRuleset> {
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
            if is_app_uid(*uid) {
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
        ipv4: build_ports_ruleset(
            PORTS_CHAIN4,
            "127.0.0.0/8",
            "icmp-port-unreachable",
            &targets,
        ),
        ipv6: build_ports_ruleset(PORTS_CHAIN6, "::1", "icmp6-port-unreachable", &targets),
        target_count: targets.len(),
    }
}
