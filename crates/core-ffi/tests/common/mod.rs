use vpnhide_core_ffi::{CoreOperationRecord, CoreSession, StateReply};

/// Accepted is durable admission, not completion. Observe publications and query
/// the same operation ID; never resubmit a command just to wait for its result.
pub async fn settled(session: &CoreSession, op: u64) -> (CoreOperationRecord, StateReply) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut state = session.read_state().await.unwrap();
        loop {
            // Capture the publication cursor BEFORE reading the receipt. A settle
            // between these reads must wake await_state instead of being missed.
            let receipt = session.operation(op).await.unwrap();
            if let Some(record) = receipt.record
                && record.settled.is_some()
            {
                // The terminal receipt may precede its final publication by
                // a scheduling turn. Wait for removal from in-flight too.
                while state.state.operations_in_flight.contains(&op) {
                    state = session
                        .await_state(state.state.epoch, state.state.revision)
                        .await
                        .unwrap();
                }
                return (record, state);
            }
            state = session
                .await_state(state.state.epoch, state.state.revision)
                .await
                .unwrap();
        }
    })
    .await
    .expect("accepted operation must settle and publish within the test deadline")
}
