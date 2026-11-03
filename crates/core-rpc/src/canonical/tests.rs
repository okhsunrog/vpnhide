use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn app(f: impl FnOnce(&mut AppConfig)) -> AppConfig {
    let mut a = AppConfig::default();
    f(&mut a);
    a
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// native=all (enabled, no per-hook overrides).
fn native_all() -> NativeRole {
    NativeRole {
        enabled: true,
        kernel: None,
        zygisk: None,
    }
}

/// A config exercising every polymorphic shape and conditional field.
fn rich() -> CanonicalConfig {
    let mut apps = BTreeMap::new();
    // java=all, native=all
    apps.insert(
        "com.a".into(),
        app(|a| {
            a.java = true;
            a.native = native_all();
        }),
    );
    // java per-hook list, native per-hook object with both families
    apps.insert(
        "com.b".into(),
        app(|a| {
            a.java = true;
            a.java_hooks = Some(vec!["hookX".into(), "hookY".into()]);
            a.native = NativeRole {
                enabled: true,
                kernel: Some(vec!["k1".into()]),
                zygisk: Some(vec!["z1".into(), "z2".into()]),
            };
        }),
    );
    // hidden only, app_hiding
    apps.insert(
        "com.c".into(),
        app(|a| {
            a.hidden = true;
            a.app_hiding = true;
        }),
    );
    // ports with a policy (single-port rule + range rule)
    apps.insert(
        "com.d".into(),
        app(|a| {
            a.ports = true;
            a.port_policy = Some(PortPolicy {
                mode: "custom".into(),
                preset: Some("torrents".into()),
                rules: vec![
                    PortRule {
                        protocol: PortProtocol::Tcp,
                        start: 443,
                        end: 443,
                    },
                    PortRule {
                        protocol: PortProtocol::Both,
                        start: 6881,
                        end: 6889,
                    },
                ],
            });
        }),
    );
    CanonicalConfig {
        version: 1,
        debug: true,
        debug_switch: false,
        apps,
        settings: Settings {
            remember_superkey: true,
            optional_features: set(&["filesystem_iface_paths"]),
            auto_hide_vpn_services: false,
            auto_hide_vpn_name: true,
            auto_hide_excluded_packages: set(&["com.x"]),
            auto_hidden_packages: set(&["com.y", "com.z"]),
        },
    }
}

#[test]
fn round_trips_through_disk() {
    let cfg = rich();
    let back = from_disk(&to_disk(&cfg)).expect("some");
    assert_eq!(back, cfg);
}

#[test]
fn encodes_the_polymorphic_and_conditional_shapes() {
    let v = to_disk(&rich());
    let apps = v["apps"].as_object().unwrap();
    // java: bool true (com.a) vs hook array (com.b)
    assert_eq!(apps["com.a"]["java"], json!(true));
    assert_eq!(apps["com.b"]["java"], json!(["hookX", "hookY"]));
    // native: bool true (com.a) vs object (com.b)
    assert_eq!(apps["com.a"]["native"], json!(true));
    assert_eq!(
        apps["com.b"]["native"],
        json!({"enabled": true, "kernel": ["k1"], "zygisk": ["z1", "z2"]})
    );
    // hidden written only when true; com.a has no hidden key, com.c does
    assert!(apps["com.a"].get("hidden").is_none());
    assert_eq!(apps["com.c"]["hidden"], json!(true));
    // portPolicy present only under ports; single-port rule omits `end`
    assert!(apps["com.a"].get("portPolicy").is_none());
    let rules = apps["com.d"]["portPolicy"]["rules"].as_array().unwrap();
    assert!(rules[0].get("end").is_none()); // 443..443
    assert_eq!(rules[1]["end"], json!(6889)); // range keeps end
}

#[test]
fn reads_a_legacy_app_written_file() {
    // Shaped exactly like StorageConfig.kt's canonicalConfigJson output.
    let raw = r#"{
      "version": 1,
      "debug": false,
      "debugSwitch": true,
      "apps": {
        "com.bool": { "java": false, "native": true, "appHiding": false, "ports": false },
        "com.arr": { "java": ["h1"], "native": ["k1", "k2"], "appHiding": true, "ports": false },
        "com.norole": { "java": false, "native": false, "appHiding": false, "ports": false }
      },
      "settings": {
        "rememberSuperkey": false,
        "optionalFeatures": [],
        "autoHideVpnServices": true,
        "autoHideVpnName": false,
        "autoHideExcludedPackages": [],
        "autoHiddenPackages": []
      }
    }"#;
    let cfg = from_disk(&serde_json::from_str(raw).unwrap()).expect("some");
    assert!(cfg.debug_switch); // explicit, not defaulted from debug
    // no-role app dropped
    assert!(!cfg.apps.contains_key("com.norole"));
    // bool native -> enabled, no overrides
    assert_eq!(cfg.apps["com.bool"].native, native_all());
    // array java -> java=true + hooks; array native -> kernel overrides
    assert_eq!(
        cfg.apps["com.arr"].java_hooks.as_deref(),
        Some(&["h1".to_string()][..])
    );
    assert_eq!(
        cfg.apps["com.arr"].native.kernel.as_deref(),
        Some(&["k1".to_string(), "k2".to_string()][..])
    );
}

#[test]
fn debug_switch_defaults_to_debug_when_absent() {
    let raw = r#"{"version":1,"debug":true,"apps":{},"settings":{}}"#;
    let cfg = from_disk(&serde_json::from_str(raw).unwrap()).unwrap();
    assert!(cfg.debug_switch, "debugSwitch absent must inherit debug");
}

#[test]
fn disabled_native_object_drops_overrides() {
    let raw = r#"{"native":{"enabled":false,"kernel":["k1"]},"appHiding":true}"#;
    let a = app_from_disk(&serde_json::from_str(raw).unwrap());
    assert_eq!(a.native, NativeRole::default());
}

#[test]
fn malformed_port_rule_disables_ports() {
    // start out of range -> whole policy dropped -> ports disabled (present-but-bad).
    let raw = r#"{"ports":true,"portPolicy":{"mode":"custom","rules":[{"start":70000}]}}"#;
    let a = app_from_disk(&serde_json::from_str(raw).unwrap());
    assert!(!a.ports);
    assert!(a.port_policy.is_none());
}

#[test]
fn newer_version_is_refused() {
    let raw = r#"{"version":2,"apps":{},"settings":{}}"#;
    assert_eq!(from_disk(&serde_json::from_str(raw).unwrap()), None);
}

#[test]
fn strict_import_rejects_corrupt_future_and_wrong_types_without_defaulting() {
    for raw in [
        "",
        "null",
        "[]",
        "{",
        r#"{"apps":[]}"#,
        r#"{"version":0}"#,
        r#"{"version":4294967297}"#,
        r#"{"version":2}"#,
        r#"{"version":"1"}"#,
        r#"{"version":1.0}"#,
        r#"{"apps":{"com.bank":{"native":42}}}"#,
        r#"{"settings":{"autoHideVpnName":"true"}}"#,
        r#"{"apps":{"com.bank":{"portPolicy":{"rules":[{"start":70000}]}}}}"#,
    ] {
        assert!(parse_import(raw).is_err(), "{raw}");
    }
    assert!(parse_import(&" ".repeat(MAX_CONFIG_BYTES + 1)).is_err());
}

#[test]
fn supported_import_retains_polymorphic_roles_and_optional_hook_families() {
    let config = parse_import(r#"{"version":1,"apps":{"com.bank":{"java":["hook"],"native":{"kernel":["hook"],"zygisk":null},"ports":true}}}"#).unwrap();
    let app = &config.apps["com.bank"];
    assert!(app.java && app.native.enabled && app.ports);
    assert_eq!(app.native.kernel.as_deref(), Some(&["hook".into()][..]));
    assert!(app.native.zygisk.is_none());
    assert_eq!(parse_import(&to_json_string(&config)).unwrap(), config);
}

#[test]
fn strict_import_keeps_legacy_rule_normalization_for_ui_roundtrip() {
    let imported = parse_import(r#"{"apps":{"com.bank":{"ports":true,"portPolicy":{"mode":"custom","rules":[{"start":8080,"protocol":"tcp"},{"start":443},{"start":8080,"protocol":"tcp"}]}}}}"#).unwrap();
    let rules = &imported.apps["com.bank"]
        .port_policy
        .as_ref()
        .unwrap()
        .rules;
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].start, 443);
    assert_eq!(rules[1].start, 8080);
}

#[test]
fn empty_explicit_port_policy_is_invalid_not_an_implicit_all_ports_policy() {
    let raw = r#"{"apps":{"com.bank":{"ports":true,"portPolicy":{"rules":[]}}}}"#;
    assert!(parse_status(raw).is_err());
    assert!(parse_import(raw).is_err());
}

#[test]
fn legacy_unknown_ui_mode_normalizes_to_custom_without_inventing_preset_rules() {
    let raw = r#"{"apps":{"com.bank":{"ports":true,"portPolicy":{"mode":"legacy-label","preset":"unknown","rules":[{"start":443}]}}}}"#;
    let imported = parse_import(raw).unwrap();
    let policy = imported.apps["com.bank"].port_policy.as_ref().unwrap();
    assert_eq!(policy.mode, "custom");
    assert_eq!(policy.preset.as_deref(), Some("unknown"));
    assert_eq!(policy.rules.len(), 1);
    assert_eq!(policy.rules[0].start, 443);
}
