//! The production socket lives in the app's own device-encrypted data dir (2.0).
//!
//! The app reaches core with an in-process RPC client and no root at all, so the socket
//! must be reachable from the app domain yet invisible to every other app:
//!
//! - **Where:** `/data/user_de/0/<pkg>/vpnhide-core.sock`. Device-encrypted storage exists
//!   at boot (before first unlock), and another app cannot traverse a foreign app's private
//!   data dir — nothing for a detector to probe. `/data/adb` is `0700` root and unusable.
//! - **Who:** owner = the dir's uid/gid, mode `0600`, the dir's SELinux label copied onto the
//!   socket ([`auth::apply_app_socket_permissions`]); peers must be root or that uid
//!   ([`auth::Allowlist::AppOwner`], re-resolved per accept).
//! - **When:** the dir appears on install and vanishes (with the socket) on uninstall or
//!   "clear data", so [`supervise`] re-binds whenever the socket is missing.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::{CoreState, ServerHandle, auth, serve_with_allowlist};

/// Env: the app's device-encrypted data dir. Set by the core module's service.sh; unset on
/// host/dev, where the plain path socket + env allowlist apply.
pub const APP_DATA_DIR_ENV: &str = "VPNHIDE_APP_DATA_DIR";
/// The socket's file name inside the app data dir (`core-rpc` `SOCKET_NAME`).
pub const SOCKET_FILE: &str = vpnhide_core_rpc::SOCKET_NAME;

const RECHECK: Duration = Duration::from_secs(5);

pub fn app_data_dir_from_env() -> Option<PathBuf> {
    std::env::var_os(APP_DATA_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub fn socket_path(app_dir: &Path) -> PathBuf {
    app_dir.join(SOCKET_FILE)
}

fn socket_present(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket())
}

/// Bind once if the app dir exists. `None` while the app is not installed.
pub fn bind(app_dir: &Path, state: &Arc<CoreState>) -> Option<ServerHandle> {
    if !app_dir.is_dir() {
        return None;
    }
    let path = socket_path(app_dir);
    match serve_with_allowlist(
        &path,
        state.clone(),
        auth::Allowlist::AppOwner(app_dir.to_path_buf()),
    ) {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("core: app socket bind failed on {}: {e}", path.display());
            None
        }
    }
}

/// Keep the app socket bound for as long as the core runs: (re)bind when the app dir exists
/// and the socket is missing (first install, reinstall, clear data). Never returns.
pub async fn supervise(app_dir: PathBuf, state: Arc<CoreState>) {
    let mut handle: Option<ServerHandle> = None;
    loop {
        if handle.is_none() || !socket_present(&socket_path(&app_dir)) {
            // Dropping the old generation first ends its connections; the path is re-bound.
            handle.take();
            handle = bind(&app_dir, &state);
        }
        tokio::time::sleep(RECHECK).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Publisher;
    use vpnhide_core_client::CoreClient;
    use vpnhide_core_rpc::StateSnapshot;

    #[tokio::test]
    async fn binds_in_app_dir_and_rebinds_after_clear_data() {
        let root = tempfile::tempdir().unwrap();
        let app_dir = root.path().join("app");
        let state = CoreState::new(Publisher::new(9, StateSnapshot::default()));

        // Not installed yet: nothing bound.
        assert!(bind(&app_dir, &state).is_none());

        std::fs::create_dir(&app_dir).unwrap();
        let task = tokio::spawn(supervise(app_dir.clone(), state.clone()));
        let path = socket_path(&app_dir);
        tokio::time::timeout(Duration::from_secs(3), async {
            while !socket_present(&path) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("bound");
        let client = CoreClient::connect(&path);
        assert_eq!(client.hello().await.unwrap(), 9);

        // "Clear data" removes the socket; the supervisor re-binds.
        std::fs::remove_file(&path).unwrap();
        tokio::time::timeout(Duration::from_secs(12), async {
            while !socket_present(&path) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("re-bound");
        let client = CoreClient::connect(&path);
        assert_eq!(client.hello().await.unwrap(), 9);
        task.abort();
    }
}
