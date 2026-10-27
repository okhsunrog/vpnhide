//! The whole path end to end: a core bound to a temp UDS socket, a typed client over
//! tarpc + JSON, and the four methods — the honest check that the contract survives the
//! real transport, not just serde in isolation.

mod common;

use std::sync::Arc;
use std::time::Duration;

use vpnhide_core_client::CoreClient;
use vpnhide_core_daemon::{CoreState, Publisher, ServerHandle, serve};
use vpnhide_core_rpc::{
    BaseRef, CanonicalConfig, ConfigEdit, OperationId, PhaseOutcome, StateSnapshot,
};

fn spawn_core(path: &std::path::Path, epoch: u64) -> (Arc<CoreState>, ServerHandle) {
    let state = CoreState::new(Publisher::new(epoch, StateSnapshot::default()));
    let handle = serve(path, state.clone()).expect("bind");
    (state, handle)
}

#[tokio::test]
async fn handshake_submit_get_and_state_since() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.sock");
    let (_state, _handle) = spawn_core(&path, 4242);

    let client = CoreClient::connect(path.clone());
    common::reserve_operation_ids(&client, 1).await;

    // Version handshake returns this run's epoch.
    assert_eq!(client.hello().await.expect("handshake"), 4242);

    // A submitted edit is accepted and becomes a queryable settled operation. The config is
    // empty, so a Replace against revision 0 bootstraps it.
    let accepted = client
        .submit_edit(
            OperationId(1),
            BaseRef { revision: 0 },
            ConfigEdit::Replace {
                config: CanonicalConfig {
                    debug: true,
                    ..Default::default()
                },
            },
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

    // The same state_since call used by the app observes the committed config.
    let published = tokio::time::timeout(Duration::from_secs(3), client.state_since(4242, 0))
        .await
        .unwrap()
        .unwrap();
    assert!(published.snapshot.config.is_some());
}

#[tokio::test]
async fn a_restarted_core_answers_an_old_epoch_cursor() {
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
        let client = CoreClient::connect(path.clone());
        common::reserve_operation_ids(&client, 1).await;
        assert_eq!(client.hello().await.expect("handshake"), 100);
        handle.shutdown_and_unlink();
    }

    // A restarted epoch answers an old cursor immediately despite its lower revision.
    let (_state, _handle) = spawn_core(&path, 200);
    let client = CoreClient::connect(path.clone());
    common::reserve_operation_ids(&client, 1).await;
    let published = tokio::time::timeout(Duration::from_secs(2), client.state_since(100, u64::MAX))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(published.epoch, 200);
    assert!(published.revision < 100);
}
