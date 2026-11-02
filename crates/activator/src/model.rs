use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use vpnhide_protocol::format_config;
use vpnhide_protocol::hook_ids::Hook;

use crate::ports::build_ports_ruleset;
use crate::{
    NO_DEFAULT_MASK, PM_READY_ATTEMPTS, PORTS_CHAIN4, PORTS_CHAIN6, PmReadyWait, Result,
    has_native_targets, is_app_uid, pm_list_packages, pm_list_users, wait_for_pm_ready,
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

pub use vpnhide_native_policy::{
    CapacityPackage, NativeCapacityExceeded, NativeHookFamily, NativeSelection,
    NativeSelectionDetail,
};

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
    )?)
}

/// Use the same pure projector that the app's local preview links.
#[cfg(test)]
pub(crate) fn native_target_set(
    cfg: &CanonicalConfig,
    resolver: &PackageUidMap,
    family: NativeHookFamily,
) -> BTreeMap<u32, (vpnhide_native_policy::HookSet, BTreeSet<String>)> {
    let selections = cfg
        .apps
        .iter()
        .map(|(package, app)| (package.clone(), app.native.clone()))
        .collect();
    vpnhide_native_policy::native_target_set(
        &selections,
        &resolver.packages,
        family,
        cfg.settings
            .optional_features
            .contains(OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS),
    )
}

pub fn project_native_with_resolver_for_family(
    cfg: &CanonicalConfig,
    resolver: &PackageUidMap,
    family: NativeHookFamily,
) -> std::result::Result<String, NativeCapacityExceeded> {
    let selections = cfg
        .apps
        .iter()
        .map(|(package, app)| (package.clone(), app.native.clone()))
        .collect();
    vpnhide_native_policy::project_native(
        cfg.debug,
        &selections,
        &resolver.packages,
        family,
        cfg.settings
            .optional_features
            .contains(OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS),
    )
}

pub fn project_native_with_resolver(
    cfg: &CanonicalConfig,
    resolver: &PackageUidMap,
) -> std::result::Result<String, NativeCapacityExceeded> {
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
