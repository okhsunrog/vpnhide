//! Process entry for the support assistant.
//!
//! The HTTP server is added with the routes. Until then the binary only
//! identifies the crate, so the workspace member is real and separate from
//! `vpnhide_core_daemon`.

fn main() {
    println!("{}", vpnhide_ai_support::CRATE_NAME);
}
