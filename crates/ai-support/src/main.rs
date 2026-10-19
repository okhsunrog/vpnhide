//! Process entry for the support assistant.
//!
//! `VPNHIDE_AI_CHECKOUTS` is a directory of versioned source trees
//! (`<version>/`, including `docs/`). `VPNHIDE_AI_BIND` defaults to
//! `127.0.0.1:8080`. The process does not activate hiding and does not shell out.

use std::net::SocketAddr;
use std::path::PathBuf;

use vpnhide_ai_support::{AppState, router};

#[tokio::main]
async fn main() {
    let checkouts = std::env::var("VPNHIDE_AI_CHECKOUTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("checkouts"));
    let bind = std::env::var("VPNHIDE_AI_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let addr: SocketAddr = bind
        .parse()
        .expect("VPNHIDE_AI_BIND must be a socket address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind VPNHIDE_AI_BIND");
    eprintln!("vpnhide-ai-support listening on {addr}");
    axum::serve(listener, router(AppState::new(checkouts)))
        .await
        .expect("server exited");
}
