//! One admitted worker's native measurement phases. The coordinator revalidates
//! the genuine worker and exact thread before EACH step; this owner never accepts
//! a caller-supplied successful grant/revocation flag.
use super::native_raw::RawLease;
use std::io::{self, Read};
use vpnhide_diagnostic_model::{
    DiagnosticError, DiagnosticThreadIdentity, NativeRawReceipt, NativeRawStage,
    NativeWorkerInstruction, NativeWorkerPhase,
};

pub(super) trait Grant: Send {
    fn activate(&mut self) -> io::Result<()>;
    fn finish(&mut self) -> io::Result<bool>;
}
impl Grant for RawLease {
    fn activate(&mut self) -> io::Result<()> {
        RawLease::activate(self)
    }
    fn finish(&mut self) -> io::Result<bool> {
        RawLease::finish(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Register,
    Raw,
    Protected,
    Failed,
}
pub(super) struct NativeExchange {
    identity: DiagnosticThreadIdentity,
    phase: Phase,
    grant: Option<Box<dyn Grant>>,
    receipt: NativeRawReceipt,
}
impl NativeExchange {
    pub(super) fn begin(
        identity: DiagnosticThreadIdentity,
        backend: Option<u32>,
    ) -> Result<(Self, NativeWorkerInstruction), DiagnosticError> {
        let Some(backend) = backend else {
            return Ok((
                Self::unsupported(identity, NativeRawReceipt::UnsupportedBackend { identity }),
                NativeWorkerInstruction::ObserveProtected,
            ));
        };
        let mut bytes = [0u8; 8];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map_err(|_| DiagnosticError::Disabled)?;
        let nonce = u64::from_ne_bytes(bytes);
        if nonce == 0 {
            return Err(DiagnosticError::Disabled);
        }
        match RawLease::arm(backend, identity.tid, identity.uid, nonce) {
            Ok(grant) => Ok((
                Self::armed(identity, Box::new(grant)),
                NativeWorkerInstruction::Register { nonce },
            )),
            // No endpoint means no grant from this call. Busy/denied/malformed
            // or uncertain control is NOT absence and must not authorize a
            // supposedly protected sample.
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((
                Self::unsupported(identity, NativeRawReceipt::CapabilityAbsent { identity }),
                NativeWorkerInstruction::ObserveProtected,
            )),
            Err(_) => Err(DiagnosticError::Disabled),
        }
    }
    pub(super) fn failed(identity: DiagnosticThreadIdentity, stage: NativeRawStage) -> Self {
        Self {
            identity,
            phase: Phase::Failed,
            grant: None,
            receipt: NativeRawReceipt::Failed {
                identity,
                stage,
                sample: None,
            },
        }
    }
    pub(super) fn identity(&self) -> DiagnosticThreadIdentity {
        self.identity
    }
    fn armed(identity: DiagnosticThreadIdentity, grant: Box<dyn Grant>) -> Self {
        Self {
            identity,
            phase: Phase::Register,
            grant: Some(grant),
            receipt: NativeRawReceipt::Failed {
                identity,
                stage: NativeRawStage::Registration,
                sample: None,
            },
        }
    }
    pub(super) fn unsupported(
        identity: DiagnosticThreadIdentity,
        receipt: NativeRawReceipt,
    ) -> Self {
        Self {
            identity,
            phase: Phase::Protected,
            grant: None,
            receipt,
        }
    }
    pub(super) fn protected_ready(&self) -> bool {
        self.phase == Phase::Protected
    }
    pub(super) fn receipt(&self) -> NativeRawReceipt {
        self.receipt.clone()
    }
    pub(super) fn step(
        &mut self,
        identity: DiagnosticThreadIdentity,
        phase: NativeWorkerPhase,
    ) -> Result<NativeWorkerInstruction, DiagnosticError> {
        if identity != self.identity {
            return Err(DiagnosticError::WrongProcess);
        }
        match (self.phase, phase) {
            (Phase::Register, NativeWorkerPhase::Registered) => {
                self.phase = Phase::Failed;
                self.receipt = NativeRawReceipt::Failed {
                    identity,
                    stage: NativeRawStage::Activation,
                    sample: None,
                };
                self.grant
                    .as_mut()
                    .ok_or(DiagnosticError::Disabled)?
                    .activate()
                    .map_err(|_| DiagnosticError::Disabled)?;
                self.phase = Phase::Raw;
                self.receipt = NativeRawReceipt::Failed {
                    identity,
                    stage: NativeRawStage::Measurement,
                    sample: None,
                };
                Ok(NativeWorkerInstruction::ObserveRaw)
            }
            (Phase::Raw, NativeWorkerPhase::RawMeasured { native }) => {
                let native = *native;
                if !native.bounded() {
                    self.phase = Phase::Failed;
                    self.receipt = NativeRawReceipt::Failed {
                        identity,
                        stage: NativeRawStage::Measurement,
                        sample: None,
                    };
                    return Err(DiagnosticError::Malformed);
                }
                self.phase = Phase::Failed;
                self.receipt = NativeRawReceipt::Failed {
                    identity,
                    stage: NativeRawStage::Revocation,
                    sample: Some(native.clone()),
                };
                let qualified = self
                    .grant
                    .as_mut()
                    .ok_or(DiagnosticError::Disabled)?
                    .finish()
                    .map_err(|_| DiagnosticError::Disabled)?;
                // finish() confirms release independently of whether the completed
                // interval remained active. Expiry loses proof, not cleanup.
                self.receipt = if qualified {
                    NativeRawReceipt::Paired {
                        identity,
                        sample: native,
                    }
                } else {
                    NativeRawReceipt::Failed {
                        identity,
                        stage: NativeRawStage::Measurement,
                        sample: Some(native),
                    }
                };
                self.grant = None;
                self.phase = Phase::Protected;
                Ok(NativeWorkerInstruction::ObserveProtected)
            }
            _ => Err(DiagnosticError::StaleSession),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use vpnhide_diagnostic_model::{NativeMeasurement, ProbeError};
    struct Fake {
        calls: Arc<Mutex<Vec<&'static str>>>,
        activate: bool,
        released: bool,
        qualified: bool,
    }
    impl Grant for Fake {
        fn activate(&mut self) -> io::Result<()> {
            self.calls.lock().unwrap().push("activate");
            if self.activate {
                Ok(())
            } else {
                Err(io::Error::other("lost activation ACK"))
            }
        }
        fn finish(&mut self) -> io::Result<bool> {
            self.calls.lock().unwrap().push("finish");
            if self.released {
                Ok(self.qualified)
            } else {
                Err(io::Error::other("unknown revoke"))
            }
        }
    }
    fn identity() -> DiagnosticThreadIdentity {
        DiagnosticThreadIdentity {
            uid: 10000,
            pid: 42,
            process_start_ticks: 100,
            tid: 43,
            thread_start_ticks: 101,
        }
    }
    fn exchange(
        activate: bool,
        released: bool,
        qualified: bool,
    ) -> (NativeExchange, Arc<Mutex<Vec<&'static str>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            NativeExchange::armed(
                identity(),
                Box::new(Fake {
                    calls: calls.clone(),
                    activate,
                    released,
                    qualified,
                }),
            ),
            calls,
        )
    }
    fn measured() -> NativeWorkerPhase {
        NativeWorkerPhase::RawMeasured {
            native: Box::new(NativeMeasurement::unavailable(ProbeError::Unsupported)),
        }
    }
    #[test]
    fn a_lost_activation_or_revocation_never_authorizes_protected_measurement() {
        for (activate, released) in [(false, true), (true, false)] {
            let (mut owner, calls) = exchange(activate, released, true);
            let first = owner.step(identity(), NativeWorkerPhase::Registered);
            if activate {
                assert_eq!(first, Ok(NativeWorkerInstruction::ObserveRaw));
                assert!(owner.step(identity(), measured()).is_err());
                assert!(matches!(
                    owner.receipt(),
                    NativeRawReceipt::Failed {
                        identity: observed,
                        stage: NativeRawStage::Revocation,
                        sample: Some(_),
                    } if observed == identity()
                ));
            } else {
                assert!(first.is_err());
            }
            assert!(!owner.protected_ready());
            assert!(
                owner
                    .step(identity(), NativeWorkerPhase::Registered)
                    .is_err()
            );
            assert_eq!(
                calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|c| **c == "activate")
                    .count(),
                1
            );
        }
    }
    #[test]
    fn expiry_retains_raw_observation_but_only_confirmed_release_permits_protected() {
        let (mut owner, _) = exchange(true, true, false);
        owner
            .step(identity(), NativeWorkerPhase::Registered)
            .unwrap();
        assert_eq!(
            owner.step(identity(), measured()),
            Ok(NativeWorkerInstruction::ObserveProtected)
        );
        assert!(owner.protected_ready());
        assert!(matches!(
            owner.receipt(),
            NativeRawReceipt::Failed {
                stage: NativeRawStage::Measurement,
                sample: Some(_),
                ..
            }
        ));
        assert!(owner.receipt().qualified_sample().is_none());
    }
    #[test]
    fn thread_reuse_wrong_order_and_repeated_commands_cannot_retarget_or_replay_grant() {
        let (mut owner, calls) = exchange(true, true, true);
        let mut reused = identity();
        reused.thread_start_ticks += 1;
        assert_eq!(
            owner.step(reused, NativeWorkerPhase::Registered),
            Err(DiagnosticError::WrongProcess)
        );
        assert!(calls.lock().unwrap().is_empty());
        assert!(owner.step(identity(), measured()).is_err());
        owner
            .step(identity(), NativeWorkerPhase::Registered)
            .unwrap();
        assert!(
            owner
                .step(identity(), NativeWorkerPhase::Registered)
                .is_err()
        );
        owner.step(identity(), measured()).unwrap();
        assert!(owner.step(identity(), measured()).is_err());
        assert_eq!(*calls.lock().unwrap(), ["activate", "finish"]);
        assert!(matches!(owner.receipt(), NativeRawReceipt::Paired { .. }));
    }
}
