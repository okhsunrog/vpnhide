//! Stable operation IDs keep the persisted crash fixtures readable. Reserve those
//! IDs explicitly through the same core allocator the app uses before submitting.
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
            if let Some(record) = client.get_operation(op).await.expect("receipt transport") {
                if record.settled.is_some() {
                    return record;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("operation settled")
}
