//! The UDS accept loop, kept apart from the tarpc plumbing so it — and its lifetime
//! rules — compile and test on the host (adapted from floppa-vpn's `rpc_listener.rs`).
//!
//! The loop runs while its [`ServerHandle`] exists and has not been shut down; dropping the
//! handle *is* a shutdown. Accepted connections are handed a [`CancellationToken`] so they
//! end with the generation that owns them — a connection that outlives its generation would
//! answer from a core instance that no longer exists.

use std::path::{Path, PathBuf};

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// Handle to a running accept loop. Drop or call [`shutdown`](Self::shutdown) to stop it.
pub struct ServerHandle {
    shutdown_tx: watch::Sender<bool>,
    connections: CancellationToken,
    socket_path: PathBuf,
    _owner: crate::instance::InstanceGuard,
}

impl ServerHandle {
    /// Stop accepting, and end the connections already accepted. The socket file is left in
    /// place — the path is shared between generations, and the next one has usually already
    /// bound it, so unlinking here would remove *its* socket.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        self.connections.cancel();
    }

    /// Stop and unlink the socket file. Only for the generation that owns the path.
    pub fn shutdown_and_unlink(self) {
        self.shutdown();
        remove_socket(&self.socket_path);
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        self.connections.cancel();
    }
}

/// Bind a Unix socket at `socket_path` and hand every accepted connection (with the
/// generation's cancellation token) to `on_connect`, until the returned handle is shut
/// down or dropped. Must be called from within a Tokio runtime.
pub fn listen(
    socket_path: &Path,
    on_connect: impl FnMut(UnixStream, CancellationToken) + Send + 'static,
) -> std::io::Result<ServerHandle> {
    listen_prepared(socket_path, |_| Ok(()), on_connect)
}

/// Preparation runs while this generation owns the bound socket, before any
/// accept task exists. Failure closes/unlinks the endpoint and releases ownership.
pub(crate) fn listen_prepared(
    socket_path: &Path,
    prepare: impl FnOnce(&Path) -> std::io::Result<()>,
    mut on_connect: impl FnMut(UnixStream, CancellationToken) + Send + 'static,
) -> std::io::Result<ServerHandle> {
    // Different state paths must not let a second live server unlink this socket.
    // Ownership is separate from the socket inode, which gets replaced on restart.
    let mut lock_path = socket_path.as_os_str().to_owned();
    lock_path.push(".lock");
    let owner = crate::instance::InstanceGuard::acquire([PathBuf::from(lock_path)])?;
    // Only the owner may remove a stale socket. Never unlink a regular file or symlink.
    if let Ok(metadata) = std::fs::symlink_metadata(socket_path) {
        use std::os::unix::fs::FileTypeExt;
        if !metadata.file_type().is_socket() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "socket path is not a socket",
            ));
        }
    }
    remove_socket(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    if let Err(error) = prepare(socket_path) {
        drop(listener);
        remove_socket(socket_path);
        return Err(error);
    }
    eprintln!("core: listening on {}", socket_path.display());

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let connections = CancellationToken::new();
    let handed_out = connections.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => match result {
                    Ok((stream, _addr)) => on_connect(stream, handed_out.clone()),
                    Err(e) => eprintln!("core: accept failed: {e}"),
                },
                changed = shutdown_rx.changed() => {
                    // `Err` means the handle was dropped — a shutdown too. Unhandled, this
                    // would become a hot loop.
                    if changed.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
    });

    Ok(ServerHandle {
        shutdown_tx,
        connections,
        socket_path: socket_path.to_path_buf(),
        _owner: owner,
    })
}

fn remove_socket(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("core: could not remove socket {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn a_second_listener_cannot_unlink_the_live_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sock");
        let handle = listen(&path, |_, _| {}).unwrap();
        assert!(listen(&path, |_, _| {}).is_err());
        UnixStream::connect(&path).await.unwrap();
        handle.shutdown_and_unlink();
        let _replacement = listen(&path, |_, _| {}).unwrap();
    }

    #[tokio::test]
    async fn an_existing_regular_file_is_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sock");
        std::fs::write(&path, b"preserve").unwrap();
        assert!(listen(&path, |_, _| {}).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"preserve");
    }

    async fn wait_until_refused(path: &Path) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while UnixStream::connect(path).await.is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the accept loop kept running");
    }

    async fn wait_for(counter: &AtomicUsize, what: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while counter.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting until {what}"));
    }

    #[tokio::test]
    async fn dropping_the_handle_ends_the_loop_and_leaves_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sock");
        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = accepted.clone();
        let handle = listen(&path, move |_s, _c| {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        UnixStream::connect(&path).await.expect("the loop is up");
        wait_for(&accepted, "the connection was handed to the callback").await;

        drop(handle);
        wait_until_refused(&path).await;
        assert!(path.exists(), "socket left for the owner to unlink");
    }

    #[tokio::test]
    async fn accepted_connections_are_cancelled_on_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.sock");
        let accepted = Arc::new(AtomicUsize::new(0));
        let ended = Arc::new(AtomicUsize::new(0));
        let (seen, done) = (accepted.clone(), ended.clone());
        let handle = listen(&path, move |_s, cancel| {
            seen.fetch_add(1, Ordering::SeqCst);
            let done = done.clone();
            tokio::spawn(async move {
                cancel.cancelled().await;
                done.fetch_add(1, Ordering::SeqCst);
            });
        })
        .unwrap();

        let _client = UnixStream::connect(&path).await.expect("the loop is up");
        wait_for(&accepted, "accepted").await;
        handle.shutdown();
        wait_for(&ended, "the accepted connection was told to stop").await;
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_owner_or_mode_setup_never_runs_handlers_and_can_retry() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("prepared.sock");
        let calls = Arc::new(AtomicUsize::new(0));
        for stage in ["owner", "mode"] {
            let count = calls.clone();
            let failed = listen_prepared(
                &socket,
                |path| {
                    // A peer can enter the kernel backlog, but no application handler
                    // may run before mandatory preparation returns successfully.
                    let _peer = std::os::unix::net::UnixStream::connect(path).unwrap();
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    assert_eq!(calls.load(Ordering::SeqCst), 0);
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        stage,
                    ))
                },
                move |_, _| {
                    count.fetch_add(1, Ordering::SeqCst);
                },
            );
            assert_eq!(
                failed.err().unwrap().kind(),
                std::io::ErrorKind::PermissionDenied
            );
            assert!(!socket.exists());
        }
        let prepared = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ready = prepared.clone();
        let count = calls.clone();
        let handle = listen_prepared(
            &socket,
            |path| {
                crate::auth::apply_private_socket_permissions(path, 0o600, None)?;
                prepared.store(true, Ordering::SeqCst);
                Ok(())
            },
            move |_, _| {
                assert!(ready.load(Ordering::SeqCst));
                count.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap();
        // A failed duplicate bind cannot change this generation's permissions.
        assert!(
            listen_prepared(&socket, |_| panic!("duplicate must not prepare"), |_, _| {}).is_err()
        );
        let _client = UnixStream::connect(&socket).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while calls.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        handle.shutdown_and_unlink();
    }
}
