//! The persistent privileged core: durable command engine, root observations and
//! authenticated UDS RPC server. App transport lives in the separate `core-client`
//! crate; ownership and lifecycle are described in `docs/core-architecture.md`.

pub mod activation;
pub mod alog;
pub mod app_socket;
pub mod auth;
pub mod autonomous;
pub mod backend;
pub mod boot;
mod command_engine;
pub mod config_edit;
pub mod config_file;
pub mod device_probes;
pub mod diagnostics;
pub mod effects;
mod export;
pub mod framework_vpn;
pub mod instance;
pub mod ipc_config;
pub mod journal;
pub mod listener;
pub mod logcat_session;
pub mod native_boot;
pub mod observer;
pub mod operation;
pub mod ownership;
pub mod payloads;
pub mod ports;
pub mod probe_cli;
pub mod publisher;
pub mod recovery;
pub mod routing_compose;
pub mod server;
pub mod service;
pub mod zygisk;
pub mod zygisk_loader;

pub use listener::{ServerHandle, listen};
pub use observer::observe;
pub use publisher::Publisher;
pub use server::{CORE_BUILD, CoreServer, CoreState, serve_connection};

use std::path::Path;
use std::sync::Arc;

/// Bind the core's socket and serve `state` on it, returning the accept-loop handle. Each
/// accepted connection is served until it closes or the handle is shut down/dropped.
/// Must be called from within a Tokio runtime.
///
/// Resolves validated endpoint configuration from env. Prefer
/// [`serve_with_allowlist`] in tests.
pub fn serve(socket_path: &Path, state: Arc<CoreState>) -> std::io::Result<ServerHandle> {
    serve_endpoint(ipc_config::EndpointConfig::from_env(socket_path)?, state)
}

/// Like [`serve`], but with an explicit peer allowlist (host tests / injected production policy).
///
/// After bind: applies socket DAC ([`auth::apply_socket_permissions`]). Each accept runs
/// [`auth::authorize`] (SO_PEERCRED + allowlist) before tarpc; rejected peers are dropped.
/// Concurrent connections are soft-capped at [`vpnhide_core_rpc::MAX_CONCURRENT_CONNECTIONS`].
pub fn serve_with_allowlist(
    socket_path: &Path,
    state: Arc<CoreState>,
    allowlist: auth::Allowlist,
) -> std::io::Result<ServerHandle> {
    serve_endpoint(
        ipc_config::EndpointConfig::explicit(socket_path, allowlist)?,
        state,
    )
}

pub fn serve_endpoint(
    config: ipc_config::EndpointConfig,
    state: Arc<CoreState>,
) -> std::io::Result<ServerHandle> {
    let socket_path = config.path().to_path_buf();
    let allowlist = config.policy();
    match &allowlist {
        auth::Allowlist::Open => {
            eprintln!("core: explicit unrestricted peer policy (library test caller)");
        }
        auth::Allowlist::Uids(set) => {
            eprintln!("core: peer allowlist uids={set:?}");
        }
        auth::Allowlist::AppOwner(dir) => {
            eprintln!("core: peer allowlist root + owner of {}", dir.display());
        }
    }
    let inflight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handle = listener::listen_prepared(
        &socket_path,
        |path| config.prepare(path),
        move |stream, cancel| {
            match auth::authorize(&stream, &allowlist) {
                Ok(cred) => {
                    eprintln!(
                        "core: accepted peer uid={} gid={} pid={}",
                        cred.uid, cred.gid, cred.pid
                    );
                }
                Err(e) => {
                    eprintln!("core: rejected peer: {e}");
                    drop(stream);
                    return;
                }
            }
            let n = inflight.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n >= vpnhide_core_rpc::MAX_CONCURRENT_CONNECTIONS {
                inflight.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                eprintln!(
                    "core: refusing connection (at MAX_CONCURRENT_CONNECTIONS={})",
                    vpnhide_core_rpc::MAX_CONCURRENT_CONNECTIONS
                );
                drop(stream);
                return;
            }
            let inflight = inflight.clone();
            let state = state.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                serve_connection(stream, state, cancel).await;
                inflight.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            });
        },
    )?;
    Ok(handle)
}

#[cfg(test)]
mod ipc_auth_tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::time::Duration;
    use vpnhide_core_client::CoreClient;
    use vpnhide_core_rpc::StateSnapshot;

    #[tokio::test]
    async fn allowlist_rejects_peer_not_on_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sock");
        let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
        // uid 1 is not the test process — fail-closed without touching process env.
        let deny = auth::Allowlist::Uids(BTreeSet::from([1u32]));
        let _handle = serve_with_allowlist(&path, state, deny).expect("listen");
        tokio::task::yield_now().await;

        let client = CoreClient::connect(&path);
        let hello = tokio::time::timeout(Duration::from_secs(2), client.hello()).await;
        match hello {
            Ok(Err(_)) | Err(_) => {}
            Ok(Ok(epoch)) => panic!("unauthorized peer must not hello, got epoch={epoch}"),
        }
    }

    #[tokio::test]
    async fn open_allowlist_accepts_same_uid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sock");
        let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
        let _handle =
            serve_with_allowlist(&path, Arc::clone(&state), auth::Allowlist::Open).expect("listen");
        tokio::task::yield_now().await;
        let client = CoreClient::connect(&path);
        let epoch = tokio::time::timeout(Duration::from_secs(5), client.hello())
            .await
            .expect("hello timed out")
            .expect("hello");
        assert_eq!(epoch, 1);
    }
}

mod vpn_networks;
