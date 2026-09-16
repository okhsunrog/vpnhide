//! Versioned observation responses shared by the in-process JNI adapter and
//! the privileged `vhhelper` executable.
//!
//! This is deliberately separate from `crates/protocol`: that crate owns the
//! kernel backend wire, while this small envelope is an app/helper boundary.

use serde::Serialize;

pub const OBSERVATION_VERSION: u32 = 1;

const KIND_CHECKS: &str = "checks";
const KIND_ROUTING: &str = "routing";
const KIND_KPM_LIST: &str = "kpm_list";

#[derive(Serialize)]
struct Success<'a, T: Serialize> {
    version: u32,
    kind: &'static str,
    status: &'static str,
    data: &'a T,
}

#[derive(Serialize)]
struct Failure {
    version: u32,
    kind: &'static str,
    status: &'static str,
    error: &'static str,
}

/// Serialize a successful response for one of the app/helper observation
/// kinds. Keeping the envelope here prevents JNI and root execution from
/// growing subtly different response formats.
pub fn success<T: Serialize>(kind: &'static str, data: &T) -> String {
    serde_json::to_string(&Success {
        version: OBSERVATION_VERSION,
        kind,
        status: "ok",
        data,
    })
    .expect("observation response serialization cannot fail")
}

/// Serialize a typed observation failure. Error strings are deliberately a
/// closed vocabulary at this boundary; details from root tools stay out of
/// stdout and logs.
pub fn error(kind: &'static str, code: &'static str) -> String {
    serde_json::to_string(&Failure {
        version: OBSERVATION_VERSION,
        kind,
        status: "error",
        error: code,
    })
    .expect("observation error serialization cannot fail")
}

pub fn checks<T: Serialize>(data: &T) -> String {
    success(KIND_CHECKS, data)
}

pub fn routing<T: Serialize>(data: &T) -> String {
    success(KIND_ROUTING, data)
}

pub fn kpm_list(raw: Option<&str>) -> String {
    let Some(raw) = raw else {
        return error(KIND_KPM_LIST, "unavailable");
    };

    // KPatch output is normally one module name per line; older wrappers have
    // also emitted rows such as `0 vpnhide loaded`. Preserve only a complete
    // safe name or the second field of a numeric row. This keeps credentials
    // and arbitrary tool diagnostics out of the structured response.
    let modules = raw
        .lines()
        .filter_map(module_name)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let data = KpmListData {
        available: true,
        modules,
    };
    success(KIND_KPM_LIST, &data)
}

#[derive(Serialize)]
struct KpmListData {
    available: bool,
    modules: Vec<String>,
}

const KPM_NON_MODULE_TOKENS: &[&str] = &[
    "active", "disabled", "enabled", "id", "inactive", "loaded", "module", "modules", "name",
    "status",
];

fn safe_module_token(token: &str) -> bool {
    !token.is_empty()
        && !token.bytes().all(|byte| byte.is_ascii_digit())
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

fn module_name(line: &str) -> Option<&str> {
    let fields = line.split_whitespace().collect::<Vec<_>>();
    let name = match fields.as_slice() {
        [name] => *name,
        [id, name, ..] if id.bytes().all(|byte| byte.is_ascii_digit()) => *name,
        _ => return None,
    };
    (safe_module_token(name) && !KPM_NON_MODULE_TOKENS.contains(&name)).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn checks_envelope_is_versioned_and_keeps_empty_data() {
        let response = checks(&Vec::<Value>::new());
        let value: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["version"], OBSERVATION_VERSION);
        assert_eq!(value["kind"], "checks");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"], serde_json::json!([]));
    }

    #[test]
    fn kpm_distinguishes_empty_from_unavailable_and_drops_unsafe_tokens() {
        let empty: Value = serde_json::from_str(&kpm_list(Some(""))).unwrap();
        assert_eq!(empty["data"]["available"], true);
        assert_eq!(empty["data"]["modules"], serde_json::json!([]));

        let unavailable: Value = serde_json::from_str(&kpm_list(None)).unwrap();
        assert_eq!(unavailable["status"], "error");
        assert_eq!(unavailable["error"], "unavailable");

        let list: Value = serde_json::from_str(&kpm_list(Some(
            "0 vpnhide loaded\nsecret=value\nother_module\n",
        )))
        .unwrap();
        assert_eq!(
            list["data"]["modules"],
            serde_json::json!(["vpnhide", "other_module"])
        );
    }

    #[test]
    fn shared_fixtures_are_valid_json_and_keep_the_wire_shapes() {
        for fixture in [
            include_str!("../../../fixtures/app-helper/checks-empty.json"),
            include_str!("../../../fixtures/app-helper/checks-unknown-status.json"),
            include_str!("../../../fixtures/app-helper/kpm-empty.json"),
            include_str!("../../../fixtures/app-helper/kpm-unavailable.json"),
            include_str!("../../../fixtures/app-helper/routing-null.json"),
            include_str!("../../../fixtures/app-helper/unsupported-version.json"),
        ] {
            let value: Value = serde_json::from_str(fixture).unwrap();
            assert!(value["version"].is_number());
            assert!(value["kind"].is_string());
            assert!(value["status"].is_string());
        }
        assert!(
            serde_json::from_str::<Value>(include_str!(
                "../../../fixtures/app-helper/malformed-truncated.json"
            ))
            .is_err()
        );
    }
}
