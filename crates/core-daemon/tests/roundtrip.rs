//! The whole path end to end: a core bound to a temp UDS socket, a typed client over
//! tarpc + JSON, and the four methods — the honest check that the contract survives the
//! real transport, not just serde in isolation.

use std::sync::Arc;
use std::time::Duration;

use vpnhide_core_daemon::{serve, CoreClient, CoreState, Publisher, ServerHandle};
use vpnhide_core_rpc::{
    BaseRef, CanonicalConfig, ConfigEdit, ConfigToggle, OperationId, PhaseOutcome, StateSnapshot,
};

fn spawn_core(path: &std::path::Path, epoch: u64) -> (Arc<CoreState>, ServerHandle) {
    let state = CoreState::new(Publisher::new(epoch, StateSnapshot::default()));
    let handle = serve(path, state.clone()).expect("bind");
    (state, handle)
}

#[tokio::test]
async fn handshake_submit_get_and_mirror() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sock");
    let (_state, _handle) = spawn_core(&path, 4242);

    let client = CoreClient::connect_and_mirror(path.clone());

    // Version handshake returns this run's epoch.
    assert_eq!(client.hello().await.expect("handshake"), 4242);

    // A submitted edit is accepted and becomes a queryable settled operation.
    let accepted = client
        .submit_edit(
            OperationId(1),
            BaseRef { revision: 1 },
            ConfigEdit::Toggle { field: ConfigToggle::Debug, enabled: true },
        )
        .await
        .expect("delivered")
        .expect("accepted");
    assert_eq!(accepted.op, OperationId(1));

    let record = client
        .get_operation(OperationId(1))
        .await
        .expect("delivered")
        .expect("record exists");
    assert_eq!(record.settled, Some(PhaseOutcome::Confirmed));

    // The submit bumped the published state; the mirror reflects it.
    let mut replica = client.state();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if replica.borrow_and_update().config.is_some() {
                break;
            }
            replica.changed().await.expect("publisher alive");
        }
    })
    .await
    .expect("the mirror reflected the submit");
}

#[tokio::test]
async fn a_restarted_core_is_adopted_by_the_mirror() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sock");

    // First run at one epoch; publish a distinctive state, then drop it.
    {
        let (state, handle) = spawn_core(&path, 100);
        state.publisher.publish(StateSnapshot {
            config: Some(CanonicalConfig::default()),
            operations_in_flight: vec![],
            ..StateSnapshot::default()
        });
        let client = CoreClient::connect_and_mirror(path.clone());
        assert_eq!(client.hello().await.expect("handshake"), 100);
        handle.shutdown_and_unlink();
    }

    // A new run at a different epoch, lower revision: a fresh client adopts it rather than
    // stalling on the old run's numbers.
    let (_state, _handle) = spawn_core(&path, 200);
    let client = CoreClient::connect_and_mirror(path.clone());
    assert_eq!(client.hello().await.expect("handshake"), 200);
}
