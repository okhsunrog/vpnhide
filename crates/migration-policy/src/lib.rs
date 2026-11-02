//! Pure direct 0.x migration policy. Core reads/binds sources and owns execution;
//! APK previews may call the same rules without acquiring privileged IO.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use vpnhide_core_rpc::{AppConfig, CanonicalConfig, NativeRole};

pub const APP_PACKAGE: &str = "dev.okhsunrog.vpnhide";
const USER_RANGE: u32 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRole {
    Kmod,
    Kpm,
    Zygisk,
    Java,
    Ports,
    Hidden,
    ObserverUids,
}
impl SourceRole {
    pub const ALL: [Self; 7] = [
        Self::Kmod,
        Self::Kpm,
        Self::Zygisk,
        Self::Java,
        Self::Ports,
        Self::Hidden,
        Self::ObserverUids,
    ];
    pub fn path(self) -> &'static str {
        match self {
            Self::Kmod => "data/adb/vpnhide_kmod/targets.txt",
            Self::Kpm => "data/adb/vpnhide_kpm/targets.txt",
            Self::Zygisk => "data/adb/vpnhide_zygisk/targets.txt",
            Self::Java => "data/adb/vpnhide_lsposed/targets.txt",
            Self::Ports => "data/adb/vpnhide_ports/observers.txt",
            Self::Hidden => "data/system/vpnhide_hidden_pkgs.txt",
            Self::ObserverUids => "data/system/vpnhide_observer_uids.txt",
        }
    }
    pub fn section(self) -> &'static str {
        match self {
            Self::Kmod => "legacy_kmod_targets",
            Self::Kpm => "legacy_kpm_targets",
            Self::Zygisk => "legacy_zygisk_targets",
            Self::Java => "legacy_lsposed_targets",
            Self::Ports => "legacy_ports_observers",
            Self::Hidden => "legacy_hidden_pkgs",
            Self::ObserverUids => "legacy_observer_uids",
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyRoles {
    pub java: bool,
    pub native: bool,
    pub app_hiding: bool,
    pub ports: bool,
    pub hidden: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyCandidate {
    pub roles: BTreeMap<String, LegacyRoles>,
    pub unresolved_observer_uids: Vec<u32>,
    pub invalid_lines: u32,
    pub complete: bool,
}
impl LegacyCandidate {
    pub fn fully_interpreted(&self) -> bool {
        self.complete && self.invalid_lines == 0 && self.unresolved_observer_uids.is_empty()
    }
    /// Cleanup is not granted by an APK preview; Core evaluates its own bound
    /// sources and requires the persisted replacement to retain all interpreted roles.
    pub fn covered_by(&self, config: &CanonicalConfig) -> bool {
        self.fully_interpreted()
            && self.roles.iter().all(|(pkg, role)| {
                config.apps.get(pkg).is_some_and(|a| {
                    (!role.java || a.java)
                        && (!role.native || a.native.enabled)
                        && (!role.app_hiding || a.app_hiding)
                        && (!role.ports || a.ports)
                        && (!role.hidden || a.hidden)
                })
            })
    }
}
pub use vpnhide_core_rpc::LegacyImportMode as ImportMode;

pub fn valid_package(pkg: &str) -> bool {
    pkg.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && pkg.contains('.')
        && pkg.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}
fn lines(raw: &str) -> impl Iterator<Item = &str> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}
pub fn parse(
    sources: &BTreeMap<SourceRole, String>,
    packages: &BTreeMap<String, Vec<u32>>,
    inventory_complete: bool,
) -> LegacyCandidate {
    let mut result = LegacyCandidate {
        roles: BTreeMap::new(),
        unresolved_observer_uids: Vec::new(),
        invalid_lines: 0,
        complete: SourceRole::ALL
            .iter()
            .all(|role| sources.contains_key(role)),
    };
    let mut observer_uids = BTreeSet::new();
    for (&role, raw) in sources {
        for line in lines(raw) {
            if role == SourceRole::ObserverUids {
                match line.parse::<u32>() {
                    Ok(uid) if uid != u32::MAX => {
                        observer_uids.insert(uid);
                    }
                    _ => result.invalid_lines = result.invalid_lines.saturating_add(1),
                }
                continue;
            }
            if !valid_package(line) {
                result.invalid_lines = result.invalid_lines.saturating_add(1);
                continue;
            }
            let roles = result.roles.entry(line.to_owned()).or_default();
            match role {
                SourceRole::Kmod | SourceRole::Kpm | SourceRole::Zygisk => roles.native = true,
                SourceRole::Java => roles.java = true,
                SourceRole::Ports => roles.ports = true,
                SourceRole::Hidden => roles.hidden = true,
                SourceRole::ObserverUids => unreachable!(),
            }
        }
    }
    if !observer_uids.is_empty() && !inventory_complete {
        result.complete = false;
    }
    for uid in observer_uids {
        let exact: Vec<_> = packages
            .iter()
            .filter(|(_, uids)| uids.contains(&uid))
            .map(|(pkg, _)| pkg)
            .collect();
        let matched = if exact.is_empty() {
            packages
                .iter()
                .filter(|(_, uids)| {
                    uids.iter()
                        .any(|known| known % USER_RANGE == uid % USER_RANGE)
                })
                .map(|(pkg, _)| pkg)
                .collect()
        } else {
            exact
        };
        if matched.is_empty() {
            result.unresolved_observer_uids.push(uid);
        }
        for pkg in matched {
            result.roles.entry(pkg.clone()).or_default().app_hiding = true;
        }
    }
    result
}

pub fn has_user_apps(config: &CanonicalConfig, self_package: &str) -> bool {
    config.apps.iter().any(|(pkg, a)| {
        pkg != self_package && (a.java || a.native.enabled || a.app_hiding || a.ports)
    })
}
pub fn with_self_target(mut config: CanonicalConfig, self_package: &str) -> CanonicalConfig {
    let app = config.apps.entry(self_package.into()).or_default();
    app.java = true;
    app.java_hooks = None;
    app.native = NativeRole {
        enabled: true,
        kernel: None,
        zygisk: None,
    };
    app.hidden = true;
    app.ports = false;
    app.port_policy = None;
    config
}
pub fn apply(
    base: &CanonicalConfig,
    candidate: &LegacyCandidate,
    mode: ImportMode,
    self_package: &str,
) -> CanonicalConfig {
    let mut next = base.clone();
    if mode == ImportMode::Replace {
        next.apps
            .retain(|pkg, a| a.hidden && base.settings.auto_hidden_packages.contains(pkg));
        for app in next.apps.values_mut() {
            *app = AppConfig {
                hidden: true,
                ..Default::default()
            };
        }
    }
    for (pkg, roles) in &candidate.roles {
        let a = next.apps.entry(pkg.clone()).or_default();
        if !a.java {
            a.java_hooks = None;
        }
        a.java |= roles.java;
        if !a.native.enabled && roles.native {
            a.native = NativeRole {
                enabled: true,
                kernel: None,
                zygisk: None,
            };
        }
        if !a.ports {
            a.port_policy = None;
        }
        a.ports |= roles.ports;
        a.app_hiding |= roles.app_hiding;
        a.hidden |= roles.hidden;
    }
    with_self_target(next, self_package)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn empty() -> BTreeMap<SourceRole, String> {
        SourceRole::ALL
            .into_iter()
            .map(|r| (r, String::new()))
            .collect()
    }
    #[test]
    fn direct_pre1_fixture_resolves_profiles_shared_uids_and_is_idempotent() {
        let raw: BTreeMap<String, String> = serde_json::from_str(include_str!(
            "../../../fixtures/migrations/pre1-sections.json"
        ))
        .unwrap();
        let sources = SourceRole::ALL
            .into_iter()
            .map(|r| (r, raw[r.section()].clone()))
            .collect();
        let packages = BTreeMap::from([
            ("com.bank.app".into(), vec![10123]),
            ("com.shared.app".into(), vec![10123]),
        ]);
        let candidate = parse(&sources, &packages, true);
        assert!(candidate.fully_interpreted());
        assert!(candidate.roles["com.bank.app"].app_hiding);
        assert!(candidate.roles["com.shared.app"].app_hiding);
        assert!(candidate.roles["com.chat.app"].ports);
        let first = apply(
            &CanonicalConfig::default(),
            &candidate,
            ImportMode::Merge,
            APP_PACKAGE,
        );
        assert_eq!(
            first,
            apply(&first, &candidate, ImportMode::Merge, APP_PACKAGE)
        );
        assert!(candidate.covered_by(&first));
        assert!(first.apps[APP_PACKAGE].native.enabled);
        assert!(!first.apps[APP_PACKAGE].ports);
    }
    #[test]
    fn unresolved_invalid_unreadable_and_incomplete_inventory_cannot_grant_cleanup() {
        let mut sources = empty();
        sources.insert(SourceRole::Kmod, "com.bank.app\nmalformed\n".into());
        sources.insert(SourceRole::ObserverUids, "10123\n-1\n4294967295\n".into());
        let candidate = parse(&sources, &BTreeMap::new(), true);
        assert_eq!(candidate.invalid_lines, 3);
        assert_eq!(candidate.unresolved_observer_uids, [10123]);
        assert!(!candidate.fully_interpreted());
        sources.remove(&SourceRole::Zygisk);
        assert!(!parse(&sources, &BTreeMap::new(), true).complete);
        sources = empty();
        sources.insert(SourceRole::ObserverUids, "10123".into());
        let packages = BTreeMap::from([("com.bank.app".into(), vec![10123])]);
        assert!(!parse(&sources, &packages, false).fully_interpreted());
        let parsed = parse(&sources, &packages, true);
        assert!(parsed.fully_interpreted());
        assert!(!parsed.covered_by(&CanonicalConfig::default()));
    }
    #[test]
    fn merge_preserves_enabled_overrides_replace_preserves_auto_hidden_marks() {
        let mut sources = empty();
        sources.insert(SourceRole::Kmod, "com.bank.app\n".into());
        sources.insert(SourceRole::Java, "com.bank.app\n".into());
        let candidate = parse(&sources, &BTreeMap::new(), true);
        let mut base = CanonicalConfig::default();
        base.settings
            .auto_hidden_packages
            .insert("com.vpn.app".into());
        base.apps.insert(
            "com.vpn.app".into(),
            AppConfig {
                hidden: true,
                ports: true,
                ..Default::default()
            },
        );
        base.apps.insert(
            "com.bank.app".into(),
            AppConfig {
                java: true,
                java_hooks: Some(vec!["custom".into()]),
                native: NativeRole {
                    enabled: true,
                    kernel: Some(vec!["custom".into()]),
                    zygisk: None,
                },
                ..Default::default()
            },
        );
        let merged = apply(&base, &candidate, ImportMode::Merge, APP_PACKAGE);
        assert_eq!(merged.apps["com.bank.app"], base.apps["com.bank.app"]);
        let replaced = apply(&base, &candidate, ImportMode::Replace, APP_PACKAGE);
        assert!(replaced.apps["com.vpn.app"].hidden);
        assert!(!replaced.apps["com.vpn.app"].ports);
        assert_eq!(replaced.apps["com.bank.app"].java_hooks, None);
        assert_eq!(replaced.apps["com.bank.app"].native.kernel, None);
    }
}
