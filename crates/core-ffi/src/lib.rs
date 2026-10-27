//! Typed UniFFI facade over the app's single persistent transport connection.
//! Core owns commands and recovery; Kotlin owns one fenced state publisher.
use std::sync::Arc;
use vpnhide_core_client::CoreClient;
use vpnhide_core_rpc as rpc;
uniffi::setup_scaffolding!();
mod api;
mod types;
pub use api::*;
pub use types::*;

#[derive(uniffi::Object)]
pub struct CoreSession {
    client: Arc<CoreClient>,
}

#[uniffi::export(async_runtime = "tokio")]
impl CoreSession {
    #[uniffi::constructor]
    pub fn new(socket_path: String) -> Arc<Self> {
        Arc::new(Self {
            client: CoreClient::connect(socket_path),
        })
    }

    pub async fn read_state(&self) -> Result<StateReply, CoreError> {
        self.await_state(0, 0).await
    }
    pub async fn await_state(&self, epoch: u64, revision: u64) -> Result<StateReply, CoreError> {
        Ok(self
            .client
            .state_since_tagged(epoch, revision)
            .await?
            .into())
    }
    pub async fn allocate_operation_id(&self) -> Result<u64, CoreError> {
        self.client
            .allocate_operation_id()
            .await?
            .map(|id| id.0)
            .map_err(|error| CoreError::Refused {
                reason: error.into(),
            })
    }
    pub async fn submit_edit(
        &self,
        op: u64,
        base_revision: u64,
        edit: CoreEdit,
    ) -> Result<SubmitReply, CoreError> {
        self.submit_command(
            op,
            CoreCommand::Edit {
                base_revision,
                edit,
            },
        )
        .await
    }
    pub async fn submit_command(
        &self,
        op: u64,
        command: CoreCommand,
    ) -> Result<SubmitReply, CoreError> {
        if op == 0 {
            return Err(CoreError::InvalidInput {
                detail: "operation ID must be allocated by core".into(),
            });
        }
        match self
            .client
            .submit_command(rpc::OperationId(op), command.into())
            .await?
        {
            Ok(accepted) => Ok(SubmitReply::Accepted {
                op: accepted.op.0,
                // Accepted work remains accepted even if these optional reads disconnect.
                record: self
                    .client
                    .get_operation(accepted.op)
                    .await
                    .ok()
                    .flatten()
                    .map(Into::into),
                state: self.read_state().await.ok().map(Box::new),
            }),
            Err(error) => Ok(SubmitReply::Refused {
                reason: error.into(),
            }),
        }
    }
    pub async fn operation(&self, op: u64) -> Result<OperationReply, CoreError> {
        Ok(OperationReply {
            record: self
                .client
                .get_operation(rpc::OperationId(op))
                .await?
                .map(Into::into),
            state: self.read_state().await.ok(),
        })
    }
    pub async fn collect_diagnostics(
        &self,
        request: CoreDiagnosticsRequest,
    ) -> Result<CoreDiagnosticsReport, CoreError> {
        Ok(self
            .client
            .collect_diagnostics(request.into())
            .await?
            .into())
    }
    pub async fn acquire_debug_lease(
        &self,
        id: u64,
        owner: CoreDebugLeaseOwner,
    ) -> Result<(), CoreError> {
        self.client
            .acquire_debug_lease(rpc::DebugLeaseId(id), owner.into())
            .await?
            .map_err(|error| CoreError::Refused {
                reason: error.into(),
            })
    }
    pub async fn release_debug_lease(&self, id: u64) -> Result<(), CoreError> {
        self.client
            .release_debug_lease(rpc::DebugLeaseId(id))
            .await?
            .map_err(|error| CoreError::Refused {
                reason: error.into(),
            })
    }
    pub async fn logcat_recording(
        &self,
        request: CoreLogcatRequest,
    ) -> Result<LogcatReply, CoreError> {
        let outcome = match request {
            CoreLogcatRequest::Status => {
                return Ok(LogcatReply::Ok {
                    session: self.client.logcat_recording_status().await?.map(Into::into),
                });
            }
            CoreLogcatRequest::Start { session } => {
                self.client
                    .start_logcat_recording(rpc::LogcatSessionId(session))
                    .await?
            }
            CoreLogcatRequest::Stop { session } => {
                self.client
                    .stop_logcat_recording(rpc::LogcatSessionId(session))
                    .await?
            }
            CoreLogcatRequest::Ack { session } => {
                self.client
                    .ack_logcat_recording(rpc::LogcatSessionId(session))
                    .await?
            }
            CoreLogcatRequest::Discard { session } => {
                return Ok(
                    match self
                        .client
                        .discard_logcat_recording(rpc::LogcatSessionId(session))
                        .await?
                    {
                        Ok(()) => LogcatReply::Ok { session: None },
                        Err(error) => LogcatReply::Refused {
                            reason: error.into(),
                        },
                    },
                );
            }
        };
        Ok(match outcome {
            Ok(session) => LogcatReply::Ok {
                session: Some(session.into()),
            },
            Err(error) => LogcatReply::Refused {
                reason: error.into(),
            },
        })
    }
}
