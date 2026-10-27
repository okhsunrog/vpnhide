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
