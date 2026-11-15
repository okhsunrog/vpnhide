use crate::{CoreDiagnosticError, CoreDiagnosticWorkerTask, CoreSession};
use vpnhide_diagnostic_model as model;

#[derive(uniffi::Record)]
pub struct CorePortsFixtureSpec {
    pub ports: Vec<u16>,
    pub challenge: Vec<u8>,
}

impl From<model::PortsFixtureSpec> for CorePortsFixtureSpec {
    fn from(spec: model::PortsFixtureSpec) -> Self {
        Self {
            ports: spec.ports.into(),
            challenge: spec.challenge.into(),
        }
    }
}

#[derive(uniffi::Enum)]
pub enum CorePortsSample {
    Echo,
    Refused,
    Unavailable,
}

impl From<model::PortsSample> for CorePortsSample {
    fn from(sample: model::PortsSample) -> Self {
        match sample {
            model::PortsSample::Echo => Self::Echo,
            model::PortsSample::Refused => Self::Refused,
            model::PortsSample::Unavailable => Self::Unavailable,
        }
    }
}

impl From<CorePortsSample> for model::PortsSample {
    fn from(sample: CorePortsSample) -> Self {
        match sample {
            CorePortsSample::Echo => Self::Echo,
            CorePortsSample::Refused => Self::Refused,
            CorePortsSample::Unavailable => Self::Unavailable,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum CorePortsWorkerPhase {
    Prepared,
    Protected { before: Vec<CorePortsSample> },
    Revoked { protected: Vec<CorePortsSample> },
    Finished { revoked: Vec<CorePortsSample> },
}

fn observations(
    values: Vec<CorePortsSample>,
) -> Result<model::PortsObservation, CoreDiagnosticError> {
    let values: [CorePortsSample; 4] = values
        .try_into()
        .map_err(|_| model::DiagnosticError::Malformed)?;
    Ok(values.map(Into::into))
}

impl TryFrom<CorePortsWorkerPhase> for model::PortsWorkerPhase {
    type Error = CoreDiagnosticError;

    fn try_from(phase: CorePortsWorkerPhase) -> Result<Self, Self::Error> {
        Ok(match phase {
            CorePortsWorkerPhase::Prepared => Self::Prepared,
            CorePortsWorkerPhase::Protected { before } => Self::Protected {
                before: observations(before)?,
            },
            CorePortsWorkerPhase::Revoked { protected } => Self::Revoked {
                protected: observations(protected)?,
            },
            CorePortsWorkerPhase::Finished { revoked } => Self::Finished {
                revoked: observations(revoked)?,
            },
        })
    }
}

#[uniffi::export]
pub fn observe_ports_fixture(
    spec: CorePortsFixtureSpec,
) -> Result<Vec<CorePortsSample>, CoreDiagnosticError> {
    let spec = model::PortsFixtureSpec {
        ports: spec
            .ports
            .try_into()
            .map_err(|_| model::DiagnosticError::Malformed)?,
        challenge: spec
            .challenge
            .try_into()
            .map_err(|_| model::DiagnosticError::Malformed)?,
    };
    if !spec.valid() {
        return Err(model::DiagnosticError::Malformed.into());
    }
    Ok(vpnhide_checks::ports_fixture::observe(&spec)
        .into_iter()
        .map(Into::into)
        .collect())
}

#[uniffi::export]
impl CoreSession {
    pub async fn ports_worker_step(
        &self,
        task: CoreDiagnosticWorkerTask,
        phase: CorePortsWorkerPhase,
    ) -> Result<Option<CorePortsFixtureSpec>, CoreDiagnosticError> {
        self.client
            .ports_worker_step(model::PortsWorkerRequest {
                session: task.session,
                core_epoch: task.core_epoch,
                phase: phase.try_into()?,
            })
            .await?
            .map(|task| task.map(Into::into))
            .map_err(Into::into)
    }
}
