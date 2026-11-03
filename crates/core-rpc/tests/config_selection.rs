use std::collections::{BTreeMap, BTreeSet};
use vpnhide_core_rpc::{
    AppConfig, CanonicalConfig, NativeRole, PortPolicy, PortProtocol, PortRule, Settings,
    config_selection::{
        AutoHideSignal, RoleSets, build_roles, hidden_packages, manual_hidden, picker_save,
        reconcile_auto_hidden, update_manual_hidden,
    },
};

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).into()).collect()
}
fn signals() -> Vec<AutoHideSignal> {
    vec![AutoHideSignal {
        package: "vpn.app".into(),
        declares_vpn_service: true,
        name_contains_vpn: false,
    }]
}
fn protected() -> AppConfig {
    AppConfig {
        java: true,
        java_hooks: Some(vec!["java_hook".into()]),
        native: NativeRole {
            enabled: true,
            kernel: Some(vec!["native_hook".into()]),
            zygisk: Some(vec![]),
        },
        ports: true,
        port_policy: Some(PortPolicy {
            mode: "custom".into(),
            preset: None,
            rules: vec![PortRule {
                start: 443,
                end: 445,
                protocol: PortProtocol::Udp,
            }],
        }),
        ..Default::default()
    }
}

#[test]
fn hidden_set_is_sorted_deduplicated_and_always_contains_self() {
    for (input, expected) in [
        (&[][..], &["self"][..]),
        (&["self"][..], &["self"][..]),
        (&["z", "a", "z"][..], &["a", "self", "z"][..]),
    ] {
        assert_eq!(hidden_packages(&set(input), "self"), set(expected));
    }
}

#[test]
fn role_rebuild_preserves_enabled_overrides_but_reenabling_resets_them() {
    let original = CanonicalConfig {
        debug: true,
        debug_switch: false,
        apps: BTreeMap::from([("bank".into(), protected())]),
        settings: Settings {
            remember_superkey: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let roles = RoleSets {
        java: set(&["bank", "new"]),
        native: set(&["bank", "new"]),
        ports: set(&["bank", "new"]),
        ..Default::default()
    };
    let same = build_roles(true, None, &roles, Some(&original));
    assert_eq!(same.apps["bank"], protected());
    assert_eq!(
        same.apps["new"],
        AppConfig {
            java: true,
            native: NativeRole {
                enabled: true,
                ..Default::default()
            },
            ports: true,
            ..Default::default()
        }
    );
    assert!(same.debug && !same.debug_switch && same.settings.remember_superkey);
    let disabled = build_roles(
        false,
        None,
        &RoleSets {
            hidden: set(&["bank"]),
            ..Default::default()
        },
        Some(&same),
    );
    let enabled = build_roles(false, None, &roles, Some(&disabled));
    assert_eq!(enabled.apps["bank"].java_hooks, None);
    assert_eq!(enabled.apps["bank"].native.kernel, None);
    assert_eq!(enabled.apps["bank"].native.zygisk, None);
    assert_eq!(enabled.apps["bank"].port_policy, None);
    assert!(build_roles(true, None, &RoleSets::default(), None).debug_switch);
    assert!(!build_roles(true, Some(false), &RoleSets::default(), None).debug_switch);
}

#[test]
fn partial_scan_preserves_unseen_auto_hidden_but_recomputes_observed_packages() {
    let base = CanonicalConfig {
        apps: BTreeMap::from([
            (
                "unseen".into(),
                AppConfig {
                    hidden: true,
                    ..Default::default()
                },
            ),
            (
                "observed".into(),
                AppConfig {
                    hidden: true,
                    ..Default::default()
                },
            ),
            (
                "manual".into(),
                AppConfig {
                    hidden: true,
                    ..Default::default()
                },
            ),
        ]),
        settings: Settings {
            auto_hidden_packages: set(&["unseen", "observed"]),
            ..Default::default()
        },
        ..Default::default()
    };
    let observed = [AutoHideSignal {
        package: "observed".into(),
        declares_vpn_service: false,
        name_contains_vpn: false,
    }];
    let partial = reconcile_auto_hidden(base.clone(), "self", &observed, true);
    assert_eq!(partial.settings.auto_hidden_packages, set(&["unseen"]));
    assert_eq!(manual_hidden(&partial, "self"), set(&["manual"]));
    assert!(!partial.apps.contains_key("observed"));
    let complete = reconcile_auto_hidden(base, "self", &observed, false);
    assert!(complete.settings.auto_hidden_packages.is_empty());
    assert!(!complete.apps.contains_key("unseen"));
}

#[test]
fn auto_hide_observations_respect_exclusions_name_opt_in_and_self() {
    let mut observed = signals();
    observed.extend([
        AutoHideSignal {
            package: "name.app".into(),
            declares_vpn_service: false,
            name_contains_vpn: true,
        },
        AutoHideSignal {
            package: "self".into(),
            declares_vpn_service: true,
            name_contains_vpn: true,
        },
    ]);
    let initial = reconcile_auto_hidden(CanonicalConfig::default(), "self", &observed, false);
    assert_eq!(initial.settings.auto_hidden_packages, set(&["vpn.app"]));
    let mut base = initial;
    base.settings.auto_hide_vpn_name = true;
    base.settings.auto_hide_excluded_packages = set(&["vpn.app"]);
    let result = reconcile_auto_hidden(base, "self", &observed, false);
    assert_eq!(result.settings.auto_hidden_packages, set(&["name.app"]));
    assert!(!result.apps.contains_key("vpn.app"));
    assert_eq!(
        reconcile_auto_hidden(result.clone(), "self", &observed, false),
        result
    );
}

#[test]
fn picker_preserves_missing_roles_and_drops_disabled_visible_policy() {
    let base = CanonicalConfig {
        debug: true,
        debug_switch: false,
        apps: BTreeMap::from([
            ("missing".into(), protected()),
            ("visible".into(), protected()),
        ]),
        ..Default::default()
    };
    let config = picker_save(
        base,
        true,
        "self",
        BTreeMap::from([(
            "visible".into(),
            AppConfig {
                java: true,
                native: NativeRole {
                    enabled: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        )]),
        &[],
        true,
    );
    assert_eq!(config.apps["missing"], protected());
    assert_eq!(config.apps["visible"].java_hooks, None);
    assert_eq!(config.apps["visible"].native.kernel, None);
    assert_eq!(config.apps["visible"].native.zygisk, None);
    assert!(!config.apps["visible"].ports);
    assert_eq!(config.apps["visible"].port_policy, None);
    assert!(config.debug && !config.debug_switch);
}

#[test]
fn manual_hidden_replaces_only_visible_manual_members_and_keeps_other_roles() {
    let base = CanonicalConfig {
        apps: BTreeMap::from([
            (
                "missing".into(),
                AppConfig {
                    hidden: true,
                    ..Default::default()
                },
            ),
            (
                "visible".into(),
                AppConfig {
                    java: true,
                    hidden: true,
                    ..Default::default()
                },
            ),
        ]),
        ..Default::default()
    };
    let result = update_manual_hidden(
        base,
        "self",
        &set(&["visible", "new"]),
        &set(&["new"]),
        &signals(),
    );
    assert_eq!(manual_hidden(&result, "self"), set(&["missing", "new"]));
    assert!(result.apps["visible"].java && !result.apps["visible"].hidden);
    assert!(result.apps["vpn.app"].hidden);
    assert_eq!(result.settings.auto_hidden_packages, set(&["vpn.app"]));
}

#[test]
fn picker_port_order_deduplicates_only_equal_protocol_ranges() {
    let mut app = protected();
    let policy = app.port_policy.as_mut().unwrap();
    policy.rules.push(PortRule {
        start: 443,
        end: 445,
        protocol: PortProtocol::Tcp,
    });
    policy.rules.push(PortRule {
        start: 443,
        end: 445,
        protocol: PortProtocol::Udp,
    });
    let config = picker_save(
        CanonicalConfig::default(),
        false,
        "self",
        BTreeMap::from([("proxy".into(), app)]),
        &[],
        false,
    );
    let rules = &config.apps["proxy"].port_policy.as_ref().unwrap().rules;
    assert_eq!(
        rules,
        &vec![
            PortRule {
                start: 443,
                end: 445,
                protocol: PortProtocol::Tcp
            },
            PortRule {
                start: 443,
                end: 445,
                protocol: PortProtocol::Udp
            }
        ]
    );
}
