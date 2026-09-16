//! App mutation lifetime supervisor. This executable is never loaded into system_server or JNI.
#[path = "../root_transport/mod.rs"]
mod root_transport;

fn main() {
    // Neither panic output nor errors may echo the command/secret input.
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reply =
        root_transport::run(&args).unwrap_or_else(|_| root_transport::Reply::error("unavailable"));
    if let Ok(json) = serde_json::to_string(&reply) {
        println!("{json}");
    }
}
