//! App mutation lifetime supervisor. This module is never loaded into
//! system_server or JNI; `vhhelper mutation` is its only production entry.
#[path = "root_transport/mod.rs"]
mod root_transport;

/// Validate the public mutation command shape before any lane or receipt is
/// opened. Numeric and identity checks remain owned by the mature transport
/// parser after it has read its bounded request.
pub fn valid_command(args: &[String]) -> bool {
    match args {
        [_, _, verb] => verb == "inspect",
        [_, _, verb, _] => verb == "adopt",
        [_, _, verb, _, _, _] => matches!(verb.as_str(), "open" | "run" | "recover"),
        _ => false,
    }
}

pub fn run(args: &[String]) -> i32 {
    let reply =
        root_transport::run(args).unwrap_or_else(|_| root_transport::Reply::error("unavailable"));
    if let Ok(json) = serde_json::to_string(&reply) {
        println!("{json}");
    }
    0
}

#[cfg(test)]
mod tests {
    use super::valid_command;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn only_known_transport_shapes_are_admitted_to_the_lane() {
        assert!(valid_command(&args(&["dir", "config", "inspect"])));
        assert!(valid_command(&args(&["dir", "config", "adopt", "session"])));
        assert!(valid_command(&args(&[
            "dir", "config", "open", "boot", "1", "session"
        ])));
        assert!(valid_command(&args(&[
            "dir", "config", "run", "boot", "session", "1"
        ])));
        assert!(valid_command(&args(&[
            "dir", "config", "recover", "boot", "session", "1"
        ])));
        assert!(!valid_command(&args(&["dir", "config", "activate-native"])));
        assert!(!valid_command(&args(&[
            "dir", "config", "unknown", "x", "y", "z"
        ])));
        assert!(!valid_command(&args(&[
            "dir", "config", "inspect", "extra"
        ])));
    }
}
