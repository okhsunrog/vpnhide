//! The durable canonical config file (core-2.0 §4 tier-1).
//!
//! The file at `/data/system/vpnhide_config.json` **is** the canonical config, not a
//! write-only projection: core writes it *and* reads it back on restart (reboot recovery),
//! and LSPosed reads it directly (no RPC — it hooks in `system_server`). So this module is
//! the disk codec between that on-disk JSON and core's clean [`CanonicalConfig`].
//!
//! # The on-disk format is polymorphic; the model is not
//!
//! The legacy format the app writes today (`StorageConfig.kt`) encodes some fields several
//! ways: `java` is a bool *or* a hook-name array; `native` is a bool, an array, *or* an
//! object; `hidden` is written only when true; `portPolicy` only when `ports`. That is a
//! wire-encoding concern, not a data concern — the normalized [`CanonicalConfig`] carries
//! the same information (`java: bool` + `java_hooks: Option`, `native: {enabled,kernel,
//! zygisk}`). This module is a faithful port of `parseCanonicalConfig` /
//! `canonicalConfigJson`, so a file written by the old app reads identically, and a file
//! core writes is read identically by LSPosed and the activator (which must not change).
//!
//! The activator's own `CanonicalConfig` (`crates/activator/model.rs`) is a **lossy,
//! deserialize-only** read model (it collapses `java` to a bool it never uses); it cannot
//! be reused here, where the round-trip must lose nothing.
//!
//! The path is a parameter, so the dev sandbox core points at an isolated file and never
//! touches the real `/data/system/vpnhide_config.json`.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use vpnhide_core_rpc::{
    AppConfig, CanonicalConfig, NativeRole, PortPolicy, PortProtocol, PortRule, Settings,
};

/// The production path of the durable canonical config.
pub const DEFAULT_CONFIG_PATH: &str = "/data/system/vpnhide_config.json";

/// Environment override for the config path — the dev sandbox core sets this to an isolated
/// file so it never writes the real one. Empty/unset falls back to [`DEFAULT_CONFIG_PATH`].
pub const CONFIG_PATH_ENV: &str = "VPNHIDE_CONFIG_PATH";

/// Resolve the config path from the environment, falling back to the production default.
pub fn config_path_from_env() -> PathBuf {
    match std::env::var(CONFIG_PATH_ENV) {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(DEFAULT_CONFIG_PATH),
    }
}

/// Outcome of loading the durable canonical config. Absent, present, and
/// unsupported schema are **distinct** — never collapse unsupported/invalid into
/// an empty default config (core-2.0 §4 / exclusive-ownership gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigLoad {
    /// File missing or blank. Core may bootstrap via Replace; this is not an error.
    Absent,
    /// Parsed `version: 1` config.
    Present(CanonicalConfig),
    /// File exists with a schema version this build must not interpret.
    UnsupportedVersion { version: u32 },
}

/// Read with full Absent / Present / Unsupported distinction.
/// Corrupt JSON or IO failures (other than NotFound) are `Err` — also not Absent.
pub fn read_status(path: &Path) -> io::Result<ConfigLoad> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ConfigLoad::Absent),
        Err(e) => return Err(e),
    };
    parse_status(&raw)
}

/// Decode on-disk canonical JSON text with the same Absent / Present / Unsupported
/// distinction as [`read_status`]. Used by the APK's root helper to hand core a config in
/// the format the app and LSPosed already speak, with one codec on the Rust side.
pub fn parse_status(raw: &str) -> io::Result<ConfigLoad> {
    if raw.trim().is_empty() {
        return Ok(ConfigLoad::Absent);
    }
    let root: Value = serde_json::from_str(raw)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("config parse: {e}")))?;
    let version = root.get("version").and_then(Value::as_u64).unwrap_or(1) as u32;
    if version > 1 {
        return Ok(ConfigLoad::UnsupportedVersion { version });
    }
    match from_disk(&root) {
        Some(cfg) => Ok(ConfigLoad::Present(cfg)),
        // from_disk only returns None for version>1, already handled above.
        None => Ok(ConfigLoad::UnsupportedVersion { version }),
    }
}

/// Convenience: `Present` → `Some`, `Absent` → `None`.
/// Unsupported schema is an **error** (not silently Absent) so callers cannot
/// treat a foreign file as "no config" and write defaults over it.
pub fn read(path: &Path) -> io::Result<Option<CanonicalConfig>> {
    match read_status(path)? {
        ConfigLoad::Absent => Ok(None),
        ConfigLoad::Present(cfg) => Ok(Some(cfg)),
        ConfigLoad::UnsupportedVersion { version } => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported config version {version}"),
        )),
    }
}

/// Serialize the config to the on-disk JSON string (the polymorphic legacy format), the same
/// text [`write_atomic`] persists. Core hands this to the activator projection, which parses
/// exactly this shape (§4: core absorbs the activator, so it feeds it the durable canon).
pub fn to_json_string(config: &CanonicalConfig) -> String {
    // to_disk builds a finite, well-formed Value, so serialization cannot fail.
    serde_json::to_string(&to_disk(config)).expect("config serializes")
}

/// Serialize and write the config atomically (temp in the same dir → `rename`), mode `0640`.
/// The rename is the commit point, so a reader (LSPosed, the activator, a restarting core)
/// never sees a half-written file. Core runs as root, so the file lands `root:root`.
pub fn write_atomic(path: &Path, config: &CanonicalConfig) -> io::Result<()> {
    let value = to_disk(config);
    let mut text = serde_json::to_string_pretty(&value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    text.push('\n');

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o640))?;
    }
    fs::rename(&tmp, path)?;
    // Best-effort durability of the rename itself; a missing dir fsync is not fatal.
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

// ---- disk JSON -> model (port of parseCanonicalConfig) ----------------------------------

fn from_disk(root: &Value) -> Option<CanonicalConfig> {
    let version = root.get("version").and_then(Value::as_u64).unwrap_or(1) as u32;
    // A newer schema than this build understands: refuse rather than misread it.
    if version > 1 {
        return None;
    }
    let debug = bool_at(root, "debug", false);
    let debug_switch = bool_at(root, "debugSwitch", debug);

    let mut apps = std::collections::BTreeMap::new();
    if let Some(obj) = root.get("apps").and_then(Value::as_object) {
        for (pkg, app_val) in obj {
            let app = app_from_disk(app_val);
            if has_any_role(&app) {
                apps.insert(pkg.clone(), app);
            }
        }
    }

    let s = root.get("settings");
    let defaults = Settings::default();
    let settings = Settings {
        remember_superkey: opt_bool_at(s, "rememberSuperkey").unwrap_or(defaults.remember_superkey),
        optional_features: string_set_at(s, "optionalFeatures"),
        auto_hide_vpn_services: opt_bool_at(s, "autoHideVpnServices")
            .unwrap_or(defaults.auto_hide_vpn_services),
        auto_hide_vpn_name: opt_bool_at(s, "autoHideVpnName")
            .unwrap_or(defaults.auto_hide_vpn_name),
        auto_hide_excluded_packages: string_set_at(s, "autoHideExcludedPackages"),
        auto_hidden_packages: string_set_at(s, "autoHiddenPackages"),
    };

    Some(CanonicalConfig {
        version,
        debug,
        debug_switch,
        apps,
        settings,
    })
}

fn app_from_disk(value: &Value) -> AppConfig {
    let (java, java_hooks) = hook_role(value.get("java"));

    let ports_requested = value.get("ports").and_then(Value::as_bool).unwrap_or(false);
    let policy_present = value.get("portPolicy").is_some_and(|v| !v.is_null());
    let port_policy = if ports_requested && policy_present {
        port_policy_from_disk(value.get("portPolicy"))
    } else {
        None
    };
    // A missing policy keeps the legacy "all ports" meaning; a present-but-malformed one
    // disables the role, so the activator never broadens it into an all-ports block.
    let ports = ports_requested && (!policy_present || port_policy.is_some());

    AppConfig {
        java,
        java_hooks,
        native: native_role_from_disk(value.get("native")),
        app_hiding: value
            .get("appHiding")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        ports,
        port_policy,
        hidden: value
            .get("hidden")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// `java`/native-list fields: bool, or a non-empty hook array (empty array == disabled).
fn hook_role(value: Option<&Value>) -> (bool, Option<Vec<String>>) {
    match value {
        Some(Value::Bool(b)) => (*b, None),
        Some(Value::Array(_)) => {
            let hooks = string_list(value);
            match hooks {
                Some(h) if !h.is_empty() => (true, Some(h)),
                _ => (false, None),
            }
        }
        _ => (false, None),
    }
}

fn native_role_from_disk(value: Option<&Value>) -> NativeRole {
    match value {
        Some(Value::Object(o)) => {
            // An object with no explicit `enabled` defaults to enabled (the activator's
            // authoritative serde default); a disabled object drops its overrides.
            let enabled = o.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if !enabled {
                return NativeRole::default();
            }
            NativeRole {
                enabled: true,
                kernel: string_list(o.get("kernel")),
                zygisk: string_list(o.get("zygisk")),
            }
        }
        _ => {
            let (enabled, hooks) = hook_role(value);
            if enabled {
                NativeRole {
                    enabled: true,
                    kernel: hooks,
                    zygisk: None,
                }
            } else {
                NativeRole::default()
            }
        }
    }
}

fn port_policy_from_disk(value: Option<&Value>) -> Option<PortPolicy> {
    let obj = value?.as_object()?;
    // Best-effort: one malformed rule drops the whole policy (→ ports disabled), never
    // unwinds the whole config.
    let rules_json = obj.get("rules")?.as_array()?;
    let mut rules = Vec::with_capacity(rules_json.len());
    for r in rules_json {
        rules.push(port_rule_from_disk(r.as_object()?)?);
    }
    let mode = obj
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("custom")
        .to_owned();
    let preset = obj
        .get("preset")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    Some(PortPolicy {
        mode,
        preset,
        rules,
    })
}

fn port_rule_from_disk(obj: &Map<String, Value>) -> Option<PortRule> {
    let start = obj.get("start").and_then(Value::as_u64)?;
    let end = match obj.get("end") {
        Some(v) => v.as_u64()?,
        None => start,
    };
    if start == 0 || start > u16::MAX as u64 || end < start || end > u16::MAX as u64 {
        return None;
    }
    let protocol = match obj
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or("both")
    {
        "tcp" => PortProtocol::Tcp,
        "udp" => PortProtocol::Udp,
        _ => PortProtocol::Both,
    };
    Some(PortRule {
        protocol,
        start: start as u16,
        end: end as u16,
    })
}

// ---- model -> disk JSON (port of canonicalConfigJson) -----------------------------------

fn to_disk(config: &CanonicalConfig) -> Value {
    let mut apps = Map::new();
    for (pkg, app) in &config.apps {
        if has_any_role(app) {
            apps.insert(pkg.clone(), app_to_disk(app));
        }
    }
    json!({
        "version": config.version,
        "debug": config.debug,
        "debugSwitch": config.debug_switch,
        "apps": Value::Object(apps),
        "settings": {
            "rememberSuperkey": config.settings.remember_superkey,
            "optionalFeatures": sorted_array(&config.settings.optional_features),
            "autoHideVpnServices": config.settings.auto_hide_vpn_services,
            "autoHideVpnName": config.settings.auto_hide_vpn_name,
            "autoHideExcludedPackages": sorted_array(&config.settings.auto_hide_excluded_packages),
            "autoHiddenPackages": sorted_array(&config.settings.auto_hidden_packages),
        },
    })
}

fn app_to_disk(app: &AppConfig) -> Value {
    let mut o = Map::new();
    o.insert(
        "java".into(),
        hook_role_value(app.java, app.java_hooks.as_deref()),
    );
    o.insert("native".into(), native_role_value(&app.native));
    o.insert("appHiding".into(), Value::Bool(app.app_hiding));
    o.insert("ports".into(), Value::Bool(app.ports));
    if app.ports {
        if let Some(policy) = &app.port_policy {
            o.insert("portPolicy".into(), port_policy_value(policy));
        }
    }
    // `hidden` is written only when true (its absence parses back to false).
    if app.hidden {
        o.insert("hidden".into(), Value::Bool(true));
    }
    Value::Object(o)
}

fn hook_role_value(enabled: bool, hooks: Option<&[String]>) -> Value {
    match (enabled, hooks) {
        (false, _) => Value::Bool(false),
        (true, None) => Value::Bool(true),
        (true, Some(h)) => json!(h),
    }
}

fn native_role_value(native: &NativeRole) -> Value {
    if !native.enabled {
        return Value::Bool(false);
    }
    if native.kernel.is_none() && native.zygisk.is_none() {
        return Value::Bool(true);
    }
    let mut o = Map::new();
    o.insert("enabled".into(), Value::Bool(true));
    if let Some(k) = &native.kernel {
        o.insert("kernel".into(), json!(k));
    }
    if let Some(z) = &native.zygisk {
        o.insert("zygisk".into(), json!(z));
    }
    Value::Object(o)
}

fn port_policy_value(policy: &PortPolicy) -> Value {
    let mut o = Map::new();
    o.insert("mode".into(), Value::String(policy.mode.clone()));
    if let Some(p) = &policy.preset {
        if !p.is_empty() {
            o.insert("preset".into(), Value::String(p.clone()));
        }
    }
    let rules: Vec<Value> = policy.rules.iter().map(port_rule_value).collect();
    o.insert("rules".into(), Value::Array(rules));
    Value::Object(o)
}

fn port_rule_value(rule: &PortRule) -> Value {
    let proto = match rule.protocol {
        PortProtocol::Both => "both",
        PortProtocol::Tcp => "tcp",
        PortProtocol::Udp => "udp",
    };
    let mut o = Map::new();
    o.insert("protocol".into(), Value::String(proto.into()));
    o.insert("start".into(), json!(rule.start));
    // `end` is omitted when it equals `start` (a single port), read back as start.
    if rule.end != rule.start {
        o.insert("end".into(), json!(rule.end));
    }
    Value::Object(o)
}

// ---- shared helpers ---------------------------------------------------------------------

/// An app with no role at all is dropped from `apps` (matches the app on both read/write).
fn has_any_role(app: &AppConfig) -> bool {
    app.java || app.native.enabled || app.app_hiding || app.ports || app.hidden
}

fn bool_at(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn opt_bool_at(v: Option<&Value>, key: &str) -> Option<bool> {
    v.and_then(|v| v.get(key)).and_then(Value::as_bool)
}

fn string_list(value: Option<&Value>) -> Option<Vec<String>> {
    let arr = value?.as_array()?;
    Some(
        arr.iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

fn string_set_at(v: Option<&Value>, key: &str) -> std::collections::BTreeSet<String> {
    v.and_then(|v| v.get(key))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn sorted_array(set: &std::collections::BTreeSet<String>) -> Value {
    Value::Array(set.iter().map(|s| Value::String(s.clone())).collect())
}

#[cfg(test)]
mod tests {
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
    fn write_then_read_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpnhide_config.json");
        let cfg = rich();
        write_atomic(&path, &cfg).unwrap();
        // Mode is 0640 and the temp file is gone.
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        assert!(!path.with_extension("json.tmp").exists());
        assert_eq!(read(&path).unwrap(), Some(cfg));
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
    fn absent_and_blank_files_are_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(&dir.path().join("nope.json")).unwrap(), None);
        assert_eq!(
            read_status(&dir.path().join("nope.json")).unwrap(),
            ConfigLoad::Absent
        );
        let blank = dir.path().join("blank.json");
        fs::write(&blank, "   \n").unwrap();
        assert_eq!(read(&blank).unwrap(), None);
        assert_eq!(read_status(&blank).unwrap(), ConfigLoad::Absent);
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.json");
        fs::write(&bad, "{ not json").unwrap();
        assert!(read(&bad).is_err());
        assert!(read_status(&bad).is_err());
    }

    #[test]
    fn unsupported_version_is_not_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg.json");
        fs::write(&path, r#"{"version":2,"apps":{},"settings":{}}"#).unwrap();
        match read_status(&path).unwrap() {
            ConfigLoad::UnsupportedVersion { version: 2 } => {}
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
        let err = read(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("unsupported config version 2"));
    }
}
