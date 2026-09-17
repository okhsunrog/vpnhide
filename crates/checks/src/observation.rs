//! Versioned observation responses shared by the in-process JNI adapter and
//! the privileged `vhhelper` executable.
//!
//! This is deliberately separate from `crates/protocol`: that crate owns the
//! kernel backend wire, while this small envelope is an app/helper boundary.

use serde::Serialize;

pub const OBSERVATION_VERSION: u32 = 1;

const KIND_CHECKS: &str = "checks";
const KIND_ROUTING: &str = "routing";
const KIND_APP_VPN_STATE: &str = "app_vpn_state";
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

pub fn app_vpn_state<T: Serialize>(data: &T) -> String {
    success(KIND_APP_VPN_STATE, data)
}

pub fn kpm_list<T: Serialize>(data: &T) -> String {
    success(KIND_KPM_LIST, data)
}

pub fn kpm_list_error(code: &'static str) -> String {
    error(KIND_KPM_LIST, code)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    #[test]
    fn shared_fixtures_are_valid_json_and_keep_the_wire_shapes() {
        for fixture in [
            include_str!("../../../fixtures/app-helper/unsupported-version.json"),
            include_str!("../../../fixtures/app-helper/routing-string-boolean.json"),
            include_str!("../../../fixtures/app-helper/kpm-malformed.json"),
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
