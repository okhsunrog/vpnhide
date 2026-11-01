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

    let record = common::settled(&client, OperationId(1)).await;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_response_reader_is_bounded_and_resumes_without_blocking_other_connections() {
    use futures::{SinkExt, StreamExt};
    use tokio_util::codec::length_delimited::LengthDelimitedCodec;
    use vpnhide_core_rpc::{VpnhideCoreRequest, VpnhideCoreResponse};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("slow.sock");
    let (state, _handle) = spawn_core(&path, 321);
    state
        .publisher
        .update(|snapshot| snapshot.startup_error = Some("x".repeat(256 * 1024)));
    let stream = tokio::net::UnixStream::connect(&path).await.unwrap();
    let framed = LengthDelimitedCodec::builder()
        .max_frame_length(vpnhide_core_rpc::MAX_FRAME_BYTES)
        .new_framed(stream);
    let mut raw = tarpc::serde_transport::new(
        framed,
        tokio_serde::formats::Json::<
            tarpc::Response<VpnhideCoreResponse>,
            tarpc::ClientMessage<VpnhideCoreRequest>,
        >::default(),
    );
    for id in 0..128 {
        let mut context = tarpc::context::current();
        context.deadline = std::time::Instant::now() + Duration::from_secs(30);
        raw.send(tarpc::ClientMessage::Request(tarpc::Request {
            context,
            id,
            message: VpnhideCoreRequest::StateSince {
                epoch: 0,
                revision: 0,
            },
        }))
        .await
        .unwrap();
    }
    // Enough large responses to fill the socket. A second connection must still
    // read receipts/state; resuming this reader must flush, not deadlock on a full
    // response queue while the driver waits for its own handler futures.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let other = CoreClient::connect(path);
    let current = tokio::time::timeout(Duration::from_secs(2), other.current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.epoch, 321);
    assert!(
        other
            .get_operation(OperationId(999))
            .await
            .unwrap()
            .is_none()
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        for _ in 0..128 {
            raw.next().await.unwrap().unwrap();
        }
    })
    .await
    .expect("slow reader can resume all bounded responses");
}
