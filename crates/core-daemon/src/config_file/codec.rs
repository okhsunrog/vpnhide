//! Pure adapter for canonical disk/interchange schema 1. This is also the current
//! LSPosed format, not a second runtime configuration model. All normalization is
//! confined here; the privileged engine operates on typed CanonicalConfig.
use super::ConfigLoad;
use serde_json::{Map, Value, json};
use std::io;
use vpnhide_core_rpc::{
    AppConfig, CanonicalConfig, NativeRole, PortPolicy, PortProtocol, PortRule, Settings,
};
/// Decode on-disk canonical JSON text with the same Absent / Present / Unsupported
/// distinction as `config_file::read_status`. The app and LSPosed still use this
/// disk interchange format; normalization stays outside the command engine.
pub fn parse_status(raw: &str) -> io::Result<ConfigLoad> {
    if raw.trim().is_empty() {
        return Ok(ConfigLoad::Absent);
    }
    let root: Value = serde_json::from_str(raw)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("config parse: {e}")))?;
    let version = validate_disk_shape(&root)?;
    if version > 1 {
        return Ok(ConfigLoad::UnsupportedVersion { version });
    }
    match from_disk(&root) {
        Some(cfg) => Ok(ConfigLoad::Present(cfg)),
        // from_disk only returns None for version>1, already handled above.
        None => Ok(ConfigLoad::UnsupportedVersion { version }),
    }
}

/// Missing legacy fields have defaults; present fields must have their documented
/// type. Otherwise a damaged policy could silently become an empty/disabled one.
fn validate_disk_shape(root: &Value) -> io::Result<u32> {
    fn invalid(field: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid canonical field: {field}"),
        )
    }
    fn check(v: &Value, keys: &[&str], accepts: fn(&Value) -> bool) -> io::Result<()> {
        for key in keys {
            if v.get(key).is_some_and(|value| !accepts(value)) {
                return Err(invalid(key));
            }
        }
        Ok(())
    }
    fn strings(v: &Value) -> bool {
        v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
    }
    fn role(v: &Value) -> bool {
        v.is_boolean() || strings(v)
    }
    if !root.is_object() {
        return Err(invalid("root"));
    }
    let version = match root.get("version") {
        None => 1,
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| invalid("version"))?,
    };
    // Do not interpret fields belonging to a future schema.
    if version > 1 {
        return Ok(version);
    }
    check(root, &["debug", "debugSwitch"], Value::is_boolean)?;
    check(root, &["apps", "settings"], Value::is_object)?;
    if let Some(settings) = root.get("settings") {
        check(
            settings,
            &["rememberSuperkey", "autoHideVpnServices", "autoHideVpnName"],
            Value::is_boolean,
        )?;
        check(
            settings,
            &[
                "optionalFeatures",
                "autoHideExcludedPackages",
                "autoHiddenPackages",
            ],
            strings,
        )?;
    }
    if let Some(apps) = root.get("apps").and_then(Value::as_object) {
        for app in apps.values() {
            if !app.is_object() {
                return Err(invalid("app"));
            }
            check(app, &["java"], role)?;
            check(app, &["appHiding", "ports", "hidden"], Value::is_boolean)?;
            if let Some(native) = app.get("native") {
                if native.is_object() {
                    check(native, &["enabled"], Value::is_boolean)?;
                    check(native, &["kernel", "zygisk"], |v| v.is_null() || strings(v))?;
                } else if !role(native) {
                    return Err(invalid("native"));
                }
            }
            if let Some(policy) = app.get("portPolicy").filter(|p| !p.is_null()) {
                if port_policy_from_disk(Some(policy)).is_none() {
                    return Err(invalid("portPolicy"));
                }
                check(policy, &["mode"], Value::is_string)?;
                check(policy, &["preset"], |v| v.is_null() || v.is_string())?;
                for rule in policy["rules"].as_array().expect("validated rules") {
                    check(rule, &["protocol"], |v| {
                        matches!(v.as_str(), Some("tcp" | "udp" | "both"))
                    })?;
                }
            }
        }
    }
    Ok(version)
}

/// Serialize the config to the on-disk JSON string (the polymorphic legacy format), the same
/// text [`write_atomic`] persists. Core hands this to the activator projection, which parses
/// exactly this shape (§4: core absorbs the activator, so it feeds it the durable canon).
pub fn to_json_string(config: &CanonicalConfig) -> String {
    // to_disk builds a finite, well-formed Value, so serialization cannot fail.
    serde_json::to_string(&to_disk(config)).expect("config serializes")
}

// ---- disk JSON -> model (port of parseCanonicalConfig) ----------------------------------

pub(super) fn from_disk(root: &Value) -> Option<CanonicalConfig> {
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

pub(super) fn app_from_disk(value: &Value) -> AppConfig {
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

pub(super) fn to_disk(config: &CanonicalConfig) -> Value {
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
