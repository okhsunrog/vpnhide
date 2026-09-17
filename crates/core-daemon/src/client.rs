//! A typed client: one cached, multiplexed connection, and a mirror loop that long-polls
//! the core's state into a watch channel (adapted from floppa-vpn's `remote.rs`).
//!
//! This is the app's read-replica (§3) on the Rust side — mirrored into a Kotlin StateFlow
//! on the other end. It never infers "the core is down" from a missing socket: on a
//! transport failure the last state stands and the loop reconnects, because the socket
//! coming and going is ordinary for a service the system starts and stops.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{watch, Mutex};
use tokio_util::codec::length_delimited::LengthDelimitedCodec;
use vpnhide_core_rpc::{
    Accepted, BaseRef, ClientHello, ConfigEdit, Handshake, OperationId, OperationRecord,
    StateSnapshot, SubmitError, VpnhideCoreClient, CALL_DEADLINE, PROTOCOL_VERSION,
    STATE_POLL_DEADLINE,
};

const RECONNECT_DELAY: Duration = Duration::from_millis(500);

pub struct CoreClient {
    socket_path: PathBuf,
    /// tarpc multiplexes, so the mirror's long poll and user calls share one connection.
    client: Mutex<Option<VpnhideCoreClient>>,
    state: watch::Sender<StateSnapshot>,
}

impl CoreClient {
    /// Connect lazily and start mirroring the core's state. The returned handle exposes the
    /// replica via [`state`](Self::state) and the typed calls.
    pub fn connect_and_mirror(socket_path: impl Into<PathBuf>) -> Arc<Self> {
        let (state, _rx) = watch::channel(StateSnapshot::default());
        let me = Arc::new(CoreClient {
            socket_path: socket_path.into(),
            client: Mutex::new(None),
            state,
        });
        tokio::spawn(mirror(me.clone()));
        me
    }

    /// A receiver over the mirrored read-replica.
    pub fn state(&self) -> watch::Receiver<StateSnapshot> {
        self.state.subscribe()
    }

    /// The version handshake — call once before relying on the connection. Returns the
    /// core's epoch on success, or an error string on protocol skew or transport failure.
    pub async fn hello(&self) -> Result<u64, String> {
        let client = self.client().await.map_err(|e| e.to_string())?;
        let hello = ClientHello {
            protocol: PROTOCOL_VERSION,
            build: env!("CARGO_PKG_VERSION").to_owned(),
        };
        match client.hello(deadline(CALL_DEADLINE), hello).await {
            Ok(Handshake::Compatible { epoch, .. }) => Ok(epoch),
            Ok(Handshake::Incompatible { core_protocol }) => Err(format!(
                "protocol skew: core={core_protocol}, app={PROTOCOL_VERSION} — update the lagging component"
            )),
            Err(e) => {
                self.drop_client().await;
                Err(e.to_string())
            }
        }
    }

    /// Submit a typed edit. `Ok(Ok)` accepted, `Ok(Err)` the core refused it, `Err` the
    /// call could not be delivered (which says nothing about whether it ran — the mirror
    /// tells the truth a moment later).
    pub async fn submit_edit(
        &self,
        op: OperationId,
        base: BaseRef,
        edit: ConfigEdit,
    ) -> Result<Result<Accepted, SubmitError>, String> {
        let client = self.client().await.map_err(|e| e.to_string())?;
        match client.submit_edit(deadline(CALL_DEADLINE), op, base, edit).await {
            Ok(result) => Ok(result),
            Err(e) => {
                self.drop_client().await;
                Err(e.to_string())
            }
        }
    }

    /// One-shot read of the core's current published state (epoch 0 asks for whatever the
    /// current run holds). Mostly for tools; screens use the mirrored [`state`](Self::state).
    pub async fn current(&self) -> Result<vpnhide_core_rpc::Published, String> {
        let client = self.client().await.map_err(|e| e.to_string())?;
        match client.state_since(deadline(CALL_DEADLINE), 0, 0).await {
            Ok(published) => Ok(published),
            Err(e) => {
                self.drop_client().await;
                Err(e.to_string())
            }
        }
    }

    /// Query an operation's record. `Err` on transport failure.
    pub async fn get_operation(&self, op: OperationId) -> Result<Option<OperationRecord>, String> {
        let client = self.client().await.map_err(|e| e.to_string())?;
        match client.get_operation(deadline(CALL_DEADLINE), op).await {
            Ok(record) => Ok(record),
            Err(e) => {
                self.drop_client().await;
                Err(e.to_string())
            }
        }
    }

    async fn client(&self) -> std::io::Result<VpnhideCoreClient> {
        let mut guard = self.client.lock().await;
        if let Some(client) = guard.as_ref() {
            return Ok(client.clone());
        }
        let stream = tokio::net::UnixStream::connect(&self.socket_path).await?;
        let framed = LengthDelimitedCodec::builder().new_framed(stream);
        let transport = tarpc::serde_transport::new(framed, tokio_serde::formats::Json::default());
        let client = VpnhideCoreClient::new(tarpc::client::Config::default(), transport).spawn();
        *guard = Some(client.clone());
        Ok(client)
    }

    async fn drop_client(&self) {
        *self.client.lock().await = None;
    }
}

fn deadline(timeout: Duration) -> tarpc::context::Context {
    let mut ctx = tarpc::context::current();
    ctx.deadline = Instant::now() + timeout;
    ctx
}

/// Keep the replica filled, forever. Reconnects on its own and never ends — a mirror that
/// stopped would quietly freeze at whatever it last saw.
async fn mirror(me: Arc<CoreClient>) {
    const NO_EPOCH: u64 = 0; // every real epoch differs, so the first answer is adopted
    let mut following = NO_EPOCH;
    let mut revision = 0u64;

    loop {
        let client = match me.client().await {
            Ok(client) => client,
            Err(_) => {
                tokio::time::sleep(RECONNECT_DELAY).await;
                continue;
            }
        };
        match client.state_since(deadline(STATE_POLL_DEADLINE), following, revision).await {
            Ok(published) => {
                if published.epoch != following {
                    // A new run of the core: its revisions start again, so adopt rather
                    // than compare numbers that are no longer comparable.
                    following = published.epoch;
                    revision = published.revision;
                    me.state.send_replace(published.snapshot);
                } else if published.revision > revision {
                    revision = published.revision;
                    me.state.send_replace(published.snapshot);
                }
                // The hold expiring returns the state we already have: loop and ask again.
            }
            Err(_) => {
                // Missing socket is not "disconnected": keep the last state, reconnect.
                me.drop_client().await;
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        }
    }
}
