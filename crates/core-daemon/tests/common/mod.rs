//! Stable operation IDs keep the persisted crash fixtures readable. Reserve those
//! IDs explicitly through the same core allocator the app uses before submitting.
#[allow(dead_code)]
pub async fn reserve_operation_ids(client: &vpnhide_core_client::CoreClient, through: u64) {
    loop {
        let allocated = client
            .allocate_operation_id()
            .await
            .expect("allocator transport")
            .expect("allocator admission");
        if allocated.0 >= through {
            break;
        }
    }
}

#[allow(dead_code)]
pub async fn settled(
    client: &vpnhide_core_client::CoreClient,
    op: vpnhide_core_rpc::OperationId,
) -> vpnhide_core_rpc::OperationRecord {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(record) = client.get_operation(op).await.expect("receipt transport")
                && record.settled.is_some()
            {
                return record;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("operation settled")
}

/// Process fixtures supply their own state/effect paths. Never inherit daemon
/// settings, fault injection or credentials from the developer's shell.
#[allow(dead_code)]
pub fn core_command() -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_vpnhide-core"));
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default());
    command
}

/// Keep fixture PATH isolated while locating host coreutils on FHS and NixOS.
#[allow(dead_code)]
pub fn link_host_tool(directory: &std::path::Path, name: &str) {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let path = std::env::var_os("PATH").expect("host PATH");
    let executable = std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|path| {
            path.metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
        .unwrap_or_else(|| panic!("host tool {name} is required by this fixture"));
    symlink(executable, directory.join(name)).expect("fixture host-tool link");
}
