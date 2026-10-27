//! One persistent, multiplexed app connection. Reconnect retries reads only; commands
//! are delivered once. A channel generation fences late replies and failures after reconnect.

use std::{
    fmt,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio_util::codec::length_delimited::LengthDelimitedCodec;
use vpnhide_core_rpc::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    Unavailable {
        generation: u64,
        detail: String,
    },
    Transport {
        generation: u64,
        detail: String,
    },
    ProtocolSkew {
        generation: u64,
        core_protocol: u32,
        app_protocol: u32,
    },
}
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { detail, .. } | Self::Transport { detail, .. } => {
                f.write_str(detail)
            }
            Self::ProtocolSkew {
                core_protocol,
                app_protocol,
                ..
            } => write!(
                f,
                "protocol skew: core={core_protocol}, app={app_protocol}; update the lagging component"
            ),
        }
    }
}
impl std::error::Error for ClientError {}

/// Only generations are comparable across sockets; core epochs are opaque identities.
#[derive(Debug)]
pub struct ClientReply<T> {
    pub generation: u64,
    pub value: T,
}

#[derive(Clone)]
struct Connection {
    generation: u64,
    epoch: u64,
    rpc: VpnhideCoreClient,
}
#[derive(Default)]
struct ConnectionState {
    generation: u64,
    connected: Option<Connection>,
}

pub struct CoreClient {
    socket_path: PathBuf,
    connection: Mutex<ConnectionState>,
}

macro_rules! call {
    ($name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty, $timeout:ident) => {
        pub async fn $name(&self, $($arg: $ty),*) -> Result<$ret, ClientError> {
            let channel = self.channel().await?;
            match channel.rpc.$name(deadline($timeout), $($arg),*).await {
                Ok(value) => Ok(value),
                Err(error) => Err(self.failed(channel.generation, error.to_string()).await),
            }
        }
    };
}

impl CoreClient {
    pub fn connect(socket_path: impl Into<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            socket_path: socket_path.into(),
            connection: Mutex::default(),
        })
    }

    pub async fn hello(&self) -> Result<u64, ClientError> {
        Ok(self.channel().await?.epoch)
    }

    pub async fn current(&self) -> Result<Published, ClientError> {
        self.state_since(0, 0).await
    }

    pub async fn state_since(&self, epoch: u64, revision: u64) -> Result<Published, ClientError> {
        self.state_since_tagged(epoch, revision)
            .await
            .map(|reply| reply.value)
    }

    /// Every publication and transport failure carries its channel generation. A late
    /// old-channel failure also cannot evict the replacement channel from the cache.
    pub async fn state_since_tagged(
        &self,
        epoch: u64,
        revision: u64,
    ) -> Result<ClientReply<Published>, ClientError> {
        let channel = self.channel().await?;
        match channel
            .rpc
            .state_since(deadline(STATE_POLL_DEADLINE), epoch, revision)
            .await
        {
            Ok(value) => Ok(ClientReply {
                generation: channel.generation,
                value,
            }),
            Err(error) => Err(self.failed(channel.generation, error.to_string()).await),
        }
    }

    /// Compatibility convenience; operation IDs must still be explicitly allocated.
    pub async fn submit_edit(
        &self,
        op: OperationId,
        base: BaseRef,
        edit: ConfigEdit,
    ) -> Result<Result<Accepted, SubmitError>, ClientError> {
        self.submit_command(op, CoreCommand::Edit { base, edit })
            .await
    }
    call!(
        get_operation(op: OperationId) -> Option<OperationRecord>,
        CALL_DEADLINE
    );
    call!(
        collect_diagnostics(request: DiagnosticsRequest) -> DiagnosticsReport,
        DIAGNOSTICS_DEADLINE
    );
    call!(allocate_operation_id() -> Result<OperationId, SubmitError>, CALL_DEADLINE);
    call!(
        submit_command(op: OperationId, command: CoreCommand) -> Result<Accepted, SubmitError>,
        CALL_DEADLINE
    );
    call!(
        acquire_debug_lease(id: DebugLeaseId, owner: DebugLeaseOwner) -> Result<(), SubmitError>,
        CALL_DEADLINE
    );
    call!(
        release_debug_lease(id: DebugLeaseId) -> Result<(), SubmitError>,
        CALL_DEADLINE
    );
    call!(
        start_logcat_recording(
            session: LogcatSessionId,
        ) -> Result<LogcatSessionStatus, LogcatRecordingError>,
        CALL_DEADLINE
    );
    call!(
        stop_logcat_recording(
            session: LogcatSessionId,
        ) -> Result<LogcatSessionStatus, LogcatRecordingError>,
        LOGCAT_STOP_DEADLINE
    );
    call!(
        ack_logcat_recording(
            session: LogcatSessionId,
        ) -> Result<LogcatSessionStatus, LogcatRecordingError>,
        CALL_DEADLINE
    );
    call!(
        discard_logcat_recording(session: LogcatSessionId) -> Result<(), LogcatRecordingError>,
        LOGCAT_STOP_DEADLINE
    );
    call!(logcat_recording_status() -> Option<LogcatSessionStatus>, CALL_DEADLINE);

    async fn channel(&self) -> Result<Connection, ClientError> {
        let mut state = self.connection.lock().await;
        if let Some(channel) = &state.connected {
            return Ok(channel.clone());
        }
        state.generation = state
            .generation
            .checked_add(1)
            .expect("connection generation exhausted");
        let generation = state.generation;
        let unavailable = |detail: String| ClientError::Unavailable { generation, detail };
        let stream = tokio::time::timeout(
            CALL_DEADLINE,
            tokio::net::UnixStream::connect(&self.socket_path),
        )
        .await
        .map_err(|error| unavailable(error.to_string()))?
        .map_err(|error| unavailable(error.to_string()))?;
        let framed = LengthDelimitedCodec::builder()
            .max_frame_length(MAX_FRAME_BYTES)
            .new_framed(stream);
        let transport = tarpc::serde_transport::new(framed, tokio_serde::formats::Json::default());
        let rpc = VpnhideCoreClient::new(tarpc::client::Config::default(), transport).spawn();
        let hello = ClientHello {
            protocol: PROTOCOL_VERSION,
            build: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let epoch = match rpc.hello(deadline(CALL_DEADLINE), hello).await {
            Ok(Handshake::Compatible { epoch, .. }) => epoch,
            Ok(Handshake::Incompatible { core_protocol }) => {
                return Err(ClientError::ProtocolSkew {
                    generation,
                    core_protocol,
                    app_protocol: PROTOCOL_VERSION,
                });
            }
            Err(error) => return Err(unavailable(error.to_string())),
        };
        let channel = Connection {
            generation,
            epoch,
            rpc,
        };
        state.connected = Some(channel.clone());
        Ok(channel)
    }

    async fn failed(&self, generation: u64, detail: String) -> ClientError {
        let mut state = self.connection.lock().await;
        if state
            .connected
            .as_ref()
            .is_some_and(|channel| channel.generation == generation)
        {
            state.connected = None;
        }
        ClientError::Transport { generation, detail }
    }
}

fn deadline(timeout: Duration) -> tarpc::context::Context {
    let mut context = tarpc::context::current();
    context.deadline = Instant::now() + timeout;
    context
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_late_old_channel_failure_does_not_disconnect_its_replacement() {
        // The older long poll may fail after another request has already reconnected.
        // Keep an idle real transport as the replacement; no RPC needs to run here.
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let framed = LengthDelimitedCodec::builder().new_framed(stream);
        let transport = tarpc::serde_transport::new(framed, tokio_serde::formats::Json::default());
        let rpc = VpnhideCoreClient::new(tarpc::client::Config::default(), transport).spawn();
        let client = CoreClient::connect("unused.sock");
        *client.connection.lock().await = ConnectionState {
            generation: 2,
            connected: Some(Connection {
                generation: 2,
                epoch: 300,
                rpc,
            }),
        };
        assert_eq!(
            client.failed(1, "old socket closed".into()).await,
            ClientError::Transport {
                generation: 1,
                detail: "old socket closed".into()
            }
        );
        assert_eq!(client.hello().await.unwrap(), 300);
        assert_eq!(client.state_generation().await, Some(2));
        client.failed(2, "current socket closed".into()).await;
        assert_eq!(client.state_generation().await, None);
    }

    impl CoreClient {
        async fn state_generation(&self) -> Option<u64> {
            self.connection
                .lock()
                .await
                .connected
                .as_ref()
                .map(|channel| channel.generation)
        }
    }
}
