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
    mut on_connect: impl FnMut(UnixStream, CancellationToken) + Send + 'static,
) -> std::io::Result<ServerHandle> {
    // A stale socket at the path blocks bind; remove it first (only here and on an owner's
    // explicit unlink, never from the accept task, which would race the next bind).
    remove_socket(socket_path);
    let listener = UnixListener::bind(socket_path)?;
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
    })
}

fn remove_socket(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("core: could not remove socket {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

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
