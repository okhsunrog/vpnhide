//! Pure role and auto-hidden projections shared by UI drafts and Core adapters.
//! Inputs describe observed inventory; no platform reads or persistence occur here.
use std::collections::{BTreeMap, BTreeSet};

use crate::{AppConfig, CanonicalConfig, NativeRole, Settings};

#[derive(Default)]
pub struct RoleSets {
    pub java: BTreeSet<String>,
    pub native: BTreeSet<String>,
    pub hidden: BTreeSet<String>,
    pub observers: BTreeSet<String>,
    pub ports: BTreeSet<String>,
}

pub struct AutoHideSignal {
    pub package: String,
    pub declares_vpn_service: bool,
    pub name_contains_vpn: bool,
}

/// Preserve overrides only while a role stays enabled, and preserve debug intent
/// independently of effective capture logging. Re-enabling starts at all hooks.
pub fn build_roles(
    debug: bool,
    debug_switch: Option<bool>,
    roles: &RoleSets,
    existing: Option<&CanonicalConfig>,
) -> CanonicalConfig {
    let packages: BTreeSet<_> = [
        &roles.java,
        &roles.native,
        &roles.hidden,
        &roles.observers,
        &roles.ports,
    ]
    .into_iter()
    .flat_map(|s| s.iter())
    .collect();
    CanonicalConfig {
        version: 1,
        debug,
        debug_switch: debug_switch
            .or(existing.map(|c| c.debug_switch))
            .unwrap_or(debug),
        settings: existing.map(|c| c.settings.clone()).unwrap_or_default(),
        apps: packages
            .into_iter()
            .map(|pkg| {
                let previous = existing.and_then(|c| c.apps.get(pkg));
                (
                    pkg.clone(),
                    AppConfig {
                        java: roles.java.contains(pkg),
                        java_hooks: previous
                            .filter(|p| p.java && roles.java.contains(pkg))
                            .and_then(|p| p.java_hooks.clone()),
                        native: if roles.native.contains(pkg) {
                            previous
                                .filter(|p| p.native.enabled)
                                .map(|p| p.native.clone())
                                .unwrap_or(NativeRole {
                                    enabled: true,
                                    ..Default::default()
                                })
                        } else {
                            NativeRole::default()
                        },
                        hidden: roles.hidden.contains(pkg),
                        app_hiding: roles.observers.contains(pkg),
                        ports: roles.ports.contains(pkg),
                        port_policy: previous
                            .filter(|p| p.ports && roles.ports.contains(pkg))
                            .and_then(|p| p.port_policy.clone()),
                    },
                )
            })
            .collect(),
    }
}

pub fn hidden_packages(existing: &BTreeSet<String>, self_pkg: &str) -> BTreeSet<String> {
    existing
        .iter()
        .cloned()
        .chain([self_pkg.to_owned()])
        .collect()
}

pub fn auto_hidden(
    signals: &[AutoHideSignal],
    settings: &Settings,
    self_pkg: &str,
) -> BTreeSet<String> {
    signals
        .iter()
        .filter(|s| {
            s.package != self_pkg
                && !settings.auto_hide_excluded_packages.contains(&s.package)
                && ((settings.auto_hide_vpn_services && s.declares_vpn_service)
                    || (settings.auto_hide_vpn_name && s.name_contains_vpn))
        })
        .map(|s| s.package.clone())
        .collect()
}

fn role_sets(config: &CanonicalConfig) -> RoleSets {
    let packages = |p: fn(&AppConfig) -> bool| {
        config
            .apps
            .iter()
            .filter(|(_, a)| p(a))
            .map(|(p, _)| p.clone())
            .collect()
    };
    RoleSets {
        java: packages(|a| a.java),
        native: packages(|a| a.native.enabled),
        hidden: packages(|a| a.hidden),
        observers: packages(|a| a.app_hiding),
        ports: packages(|a| a.ports),
    }
}

pub fn manual_hidden(config: &CanonicalConfig, self_pkg: &str) -> BTreeSet<String> {
    config
        .apps
        .iter()
        .filter(|(pkg, app)| {
            app.hidden && *pkg != self_pkg && !config.settings.auto_hidden_packages.contains(*pkg)
        })
        .map(|(pkg, _)| pkg.clone())
        .collect()
}

/// A partial profile scan cannot revoke auto-hidden membership of unseen packages.
/// Manual hiding and independent roles survive reconciliation.
pub fn reconcile_auto_hidden(
    mut config: CanonicalConfig,
    self_pkg: &str,
    signals: &[AutoHideSignal],
    partial: bool,
) -> CanonicalConfig {
    let mut roles = role_sets(&config);
    let manual: BTreeSet<_> = roles
        .hidden
        .difference(&config.settings.auto_hidden_packages)
        .cloned()
        .collect();
    let mut automatic = auto_hidden(signals, &config.settings, self_pkg);
    if partial {
        let seen: BTreeSet<_> = signals.iter().map(|s| &s.package).collect();
        automatic.extend(
            config
                .settings
                .auto_hidden_packages
                .iter()
                .filter(|p| !seen.contains(p))
                .cloned(),
        );
    }
    roles.hidden = hidden_packages(&manual.union(&automatic).cloned().collect(), self_pkg);
    config.settings.auto_hidden_packages = automatic;
    build_roles(config.debug, None, &roles, Some(&config))
}

pub fn update_manual_hidden(
    config: CanonicalConfig,
    self_pkg: &str,
    visible: &BTreeSet<String>,
    selected: &BTreeSet<String>,
    signals: &[AutoHideSignal],
) -> CanonicalConfig {
    let next: BTreeSet<_> = manual_hidden(&config, self_pkg)
        .difference(visible)
        .cloned()
        .chain(selected.iter().cloned())
        .collect();
    let mut roles = role_sets(&config);
    roles.hidden = hidden_packages(&next, self_pkg);
    let manual = build_roles(config.debug, None, &roles, Some(&config));
    reconcile_auto_hidden(manual, self_pkg, signals, false)
}

/// Visible rows are explicit role selections. Missing rows keep their existing
/// roles/overrides, including packages in profiles unavailable to this scan.
pub fn picker_save(
    mut base: CanonicalConfig,
    debug: bool,
    self_pkg: &str,
    selections: BTreeMap<String, AppConfig>,
    signals: &[AutoHideSignal],
    partial: bool,
) -> CanonicalConfig {
    base.debug = debug;
    let mut roles = role_sets(&base);
    for (pkg, value) in &selections {
        for (set, enabled) in [
            (&mut roles.java, value.java),
            (&mut roles.native, value.native.enabled),
            (&mut roles.observers, value.app_hiding),
            (&mut roles.ports, value.ports),
        ] {
            if enabled {
                set.insert(pkg.clone());
            } else {
                set.remove(pkg);
            }
        }
    }
    roles.java.insert(self_pkg.into());
    roles.native.insert(self_pkg.into());
    roles.hidden = hidden_packages(
        &roles
            .hidden
            .difference(&base.settings.auto_hidden_packages)
            .cloned()
            .collect(),
        self_pkg,
    );
    let mut config = build_roles(debug, None, &roles, Some(&base));
    for (pkg, value) in selections {
        let Some(app) = config.apps.get_mut(&pkg) else {
            continue;
        };
        if value.java {
            app.java_hooks = value.java_hooks.filter(|v| !v.is_empty());
        }
        if value.native.enabled {
            app.native = value.native;
        }
        if value.ports {
            app.port_policy = value.port_policy.map(|mut p| {
                crate::canonical::normalize_port_policy(&mut p);
                p
            });
        }
    }
    reconcile_auto_hidden(config, self_pkg, signals, partial)
}
