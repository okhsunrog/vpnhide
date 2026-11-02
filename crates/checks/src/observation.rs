//! Versioned observation responses shared by the in-process JNI adapter and
//! core (`vpnhide-core observe …`).
//!
//! This is deliberately separate from `crates/protocol`: that crate owns the
//! kernel backend wire, while this small envelope is an app/helper boundary.

use serde::Serialize;

pub const OBSERVATION_VERSION: u32 = 1;

const KIND_CHECKS: &str = "checks";

#[derive(Serialize)]
struct Success<'a, T: Serialize> {
    version: u32,
    kind: &'static str,
    status: &'static str,
    data: &'a T,
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

pub fn checks<T: Serialize>(data: &T) -> String {
    success(KIND_CHECKS, data)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    #[test]
    fn shared_fixtures_are_valid_json_and_keep_the_wire_shapes() {
        let value: Value = serde_json::from_str(include_str!(
            "../../../fixtures/core-probes/unsupported-version.json"
        ))
        .unwrap();
        assert!(value["version"].is_number());
        assert!(value["kind"].is_string());
        assert!(value["status"].is_string());
        assert!(
            serde_json::from_str::<Value>(include_str!(
                "../../../fixtures/core-probes/malformed-truncated.json"
            ))
            .is_err()
        );
    }
}
