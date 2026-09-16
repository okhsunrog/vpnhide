//! App mutation lifetime supervisor. This module is never loaded into
//! system_server or JNI; `vhhelper mutation` is its only production entry.
#[path = "root_transport/mod.rs"]
mod root_transport;

pub fn run(args: &[String]) -> i32 {
    if args.len() == 2 && args[0] == "activate" && args[1] == "native" {
        let code = match vpnhide_activator::activate_native_companion() {
            Ok(code) => code,
            Err(error) => {
                eprintln!("native activation refused: {error}");
                1
            }
        };
        return code;
    }
    let reply =
        root_transport::run(args).unwrap_or_else(|_| root_transport::Reply::error("unavailable"));
    if let Ok(json) = serde_json::to_string(&reply) {
        println!("{json}");
    }
    0
}
