//! vpnhide 2.0 persistent root core: a UDS tarpc server for the `core-rpc` contract, plus
//! a typed client. Skeleton for core-2.0 step 2 — the publish/lifecycle machinery is real;
//! the domain (config/operation/observation) is still placeholder (see `core-rpc`).

pub mod client;
pub mod activation;
pub mod config_edit;
pub mod config_file;
pub mod listener;
pub mod observer;
pub mod operation;
pub mod publisher;
pub mod server;

pub use client::CoreClient;
pub use listener::{listen, ServerHandle};
pub use observer::observe;
pub use publisher::Publisher;
pub use server::{serve_connection, CoreServer, CoreState, CORE_BUILD};

use std::path::Path;
use std::sync::Arc;

/// Bind the core's socket and serve `state` on it, returning the accept-loop handle. Each
/// accepted connection is served until it closes or the handle is shut down/dropped.
/// Must be called from within a Tokio runtime.
pub fn serve(socket_path: &Path, state: Arc<CoreState>) -> std::io::Result<ServerHandle> {
    listen(socket_path, move |stream, cancel| {
        serve_connection(stream, state.clone(), cancel)
    })
}
