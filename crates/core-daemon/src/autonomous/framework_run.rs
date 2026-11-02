//! Core owns source authorization, phase ordering, attribution, evidence and cleanup.
use super::framework_fixture::FrameworkStore;
use std::os::unix::fs::MetadataExt;
use std::{
    fs,
    io::{self, Read},
    path::Path,
    time::{Duration, Instant},
};
use vpnhide_diagnostic_model::{DiagnosticError, framework::*};
use vpnhide_loader_ipc::image;

pub(super) struct FrameworkRun {
    envelope: FrameworkEnvelope,
    phases: Vec<FrameworkPhaseEvidence>,
    last: Option<FrameworkWorkerRequest>,
    finished: bool,
    closed: bool,
}
fn failed(_: impl std::fmt::Display) -> DiagnosticError {
    DiagnosticError::Disabled
}
fn boot_ms() -> io::Result<u64> {
    let mut t: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut t) } != 0 || t.tv_sec < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(t.tv_sec as u64 * 1000 + t.tv_nsec as u64 / 1_000_000)
}
fn system_server(ack: &FrameworkAcknowledgement) -> bool {
    let root = Path::new("/proc");
    let dir = root.join(ack.system_server_pid.to_string());
    fs::metadata(&dir).is_ok_and(|m| m.uid() == 1000)
        && image::bounded_text(&dir.join("cmdline"), 512)
            .is_ok_and(|v| v.split('\0').next() == Some("system_server"))
        && image::process_identity(root, ack.system_server_pid)
            .is_ok_and(|p| p.start_ticks == ack.system_server_start_ticks)
}
impl FrameworkRun {
    #[cfg(target_os = "android")]
    pub fn recover_orphan(epoch: u64) -> Result<(), DiagnosticError> {
        let boot = image::bounded_text(Path::new(crate::native_boot::BOOT_ID_PATH), 128)
            .map_err(failed)?;
        FrameworkStore::production()
            .map_err(failed)?
            .reconcile_orphan(epoch, boot.trim(), boot_ms().map_err(failed)?)
            .map_err(failed)
    }
    pub fn new(
        session: u64,
        epoch: u64,
        worker: (u32, u32, u64),
        interface: String,
        mtu: u32,
        hash: String,
    ) -> Result<Self, DiagnosticError> {
        let mut entropy = [0u8; 32];
        fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut entropy))
            .map_err(failed)?;
        let nonce = entropy.iter().map(|b| format!("{b:02x}")).collect();
        let random_id =
            u32::from_le_bytes(entropy[..4].try_into().expect("four bytes")) & 0x3fff_ffff;
        let envelope = FrameworkEnvelope {
            version: FRAMEWORK_PROTOCOL,
            session,
            core_epoch: epoch,
            boot_id: image::bounded_text(Path::new(crate::native_boot::BOOT_ID_PATH), 128)
                .map_err(failed)?
                .trim()
                .into(),
            nonce,
            expected_hook_sha256: hash,
            worker_uid: worker.0,
            worker_pid: worker.1,
            worker_start_ticks: worker.2,
            expires_boot_ms: boot_ms()
                .map_err(failed)?
                .checked_add(20_000)
                .ok_or(DiagnosticError::Expired)?,
            generation: 0,
            phase: FrameworkPhase::Prepared,
            fixture: FrameworkFixtureSpec {
                vpn_net_id: 0x4000_0000 + (random_id & !1),
                ordinary_net_id: 0x4000_0001 + (random_id & !1),
                interface_name: interface,
                mtu,
                ordinary_interface_name: format!("ipsec{:010x}", session & 0xff_ffff_ffff),
                ordinary_mtu: 1519,
            },
        };
        if !envelope.valid() {
            return Err(DiagnosticError::Malformed);
        }
        Ok(Self {
            envelope,
            phases: Vec::new(),
            last: None,
            finished: false,
            closed: false,
        })
    }
    fn acknowledgement(
        &self,
        store: &FrameworkStore,
        validate: fn(&FrameworkAcknowledgement) -> bool,
    ) -> Result<FrameworkAcknowledgement, DiagnosticError> {
        let until = Instant::now() + Duration::from_millis(800);
        loop {
            if boot_ms().map_err(failed)? >= self.envelope.expires_boot_ms {
                return Err(DiagnosticError::Expired);
            }
            if store.current().map_err(failed)?.as_ref() != Some(&self.envelope) {
                return Err(DiagnosticError::StaleSession);
            }
            if let Some(ack) = store.acknowledgement(&self.envelope).map_err(failed)? {
                if !validate(&ack) {
                    return Err(DiagnosticError::WrongProcess);
                }
                if self.phases.first().is_some_and(|first| {
                    first.acknowledgement.sources != ack.sources
                        || first.acknowledgement.system_server_pid != ack.system_server_pid
                        || first.acknowledgement.system_server_start_ticks
                            != ack.system_server_start_ticks
                }) {
                    return Err(DiagnosticError::StaleSession);
                }
                return Ok(ack);
            }
            if Instant::now() >= until {
                return Err(DiagnosticError::Disabled);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn step(
        &mut self,
        request: FrameworkWorkerRequest,
    ) -> Result<Option<FrameworkWorkerTask>, DiagnosticError> {
        let store = FrameworkStore::production().map_err(failed)?;
        self.step_with(request, &store, system_server)
    }
    fn step_with(
        &mut self,
        request: FrameworkWorkerRequest,
        store: &FrameworkStore,
        validate: fn(&FrameworkAcknowledgement) -> bool,
    ) -> Result<Option<FrameworkWorkerTask>, DiagnosticError> {
        if self.closed {
            return if self.finished && self.last.as_ref() == Some(&request) {
                Ok(None)
            } else {
                Err(DiagnosticError::Expired)
            };
        }
        if request.session != self.envelope.session
            || request.core_epoch != self.envelope.core_epoch
            || request.hook_sha256 != self.envelope.expected_hook_sha256
        {
            return Err(DiagnosticError::StaleSession);
        }
        if request.previous.as_ref().is_some_and(|v| !v.bounded()) {
            return Err(DiagnosticError::Malformed);
        }
        if self.last.as_ref() != Some(&request) {
            if self.finished {
                return Err(DiagnosticError::StaleSession);
            }
            self.advance(store, &request, validate)?;
            self.last = Some(request.clone());
        }
        if self.finished {
            return Ok(None);
        }
        let ack = self.acknowledgement(store, validate)?;
        Ok(Some(FrameworkWorkerTask {
            phase: request.phase,
            vpn_handle: ack.sources.vpn_handle,
            ordinary_handle: ack.sources.ordinary_handle,
            vpn_marker: self.envelope.fixture.interface_name.clone(),
            ordinary_marker: self.envelope.fixture.ordinary_interface_name.clone(),
        }))
    }
    fn advance(
        &mut self,
        store: &FrameworkStore,
        request: &FrameworkWorkerRequest,
        validate: fn(&FrameworkAcknowledgement) -> bool,
    ) -> Result<(), DiagnosticError> {
        let ordinal = match request.phase {
            FrameworkWorkerPhase::Prepared => 0,
            FrameworkWorkerPhase::Raw => 1,
            FrameworkWorkerPhase::Protected => 2,
            FrameworkWorkerPhase::Revoked => 3,
            FrameworkWorkerPhase::Finished => 4,
        };
        if ordinal == 0 {
            if self.last.is_some() || request.previous.is_some() {
                return Err(DiagnosticError::StaleSession);
            }
            store
                .reconcile_orphan(
                    self.envelope.core_epoch,
                    &self.envelope.boot_id,
                    boot_ms().map_err(failed)?,
                )
                .map_err(failed)?;
            store.publish(None, &self.envelope).map_err(failed)?;
        } else {
            if self.last.is_none() || ordinal != self.envelope.generation + 1 {
                return Err(DiagnosticError::StaleSession);
            }
            let observation = request.previous.clone().ok_or(DiagnosticError::Malformed)?;
            let acknowledgement = self.acknowledgement(store, validate)?;
            let evidence = FrameworkPhaseEvidence {
                envelope: self.envelope.clone(),
                acknowledgement,
                observation,
            };
            if let Some(phase) = request.phase.authority_phase() {
                let next = self.envelope.for_phase(phase);
                store.publish(Some(&self.envelope), &next).map_err(failed)?;
                self.envelope = next;
            } else {
                store.revoke(&self.envelope).map_err(failed)?;
                self.finished = true;
            }
            self.phases.push(evidence);
        }
        Ok(())
    }
    pub fn cleanup(&mut self) -> (ScopedFrameworkResult, bool) {
        // A step admitted before cancellation may acquire this mutex afterward.
        // Terminalize before I/O so it can never republish a cleaned authorization.
        self.closed = true;
        let cleaned = FrameworkStore::production()
            .and_then(|store| self.cleanup_store(&store))
            .unwrap_or(false);
        (evaluate_framework(&self.phases, cleaned), cleaned)
    }
    fn cleanup_store(&mut self, store: &FrameworkStore) -> io::Result<bool> {
        self.closed = true;
        match store.current()? {
            None => Ok(true),
            Some(current)
                if current.session == self.envelope.session
                    && current.core_epoch == self.envelope.core_epoch
                    && current.nonce == self.envelope.nonce =>
            {
                store.revoke(&current)?;
                Ok(true)
            }
            Some(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    fn run() -> FrameworkRun {
        let mut envelope: FrameworkEnvelope = serde_json::from_str(include_str!(
            "../../../../fixtures/diagnostics/framework-envelope.json"
        ))
        .unwrap();
        envelope.expires_boot_ms = boot_ms().unwrap() + 20_000;
        FrameworkRun {
            envelope,
            phases: vec![],
            last: None,
            finished: false,
            closed: false,
        }
    }
    fn request(run: &FrameworkRun, phase: FrameworkWorkerPhase) -> FrameworkWorkerRequest {
        FrameworkWorkerRequest {
            session: run.envelope.session,
            core_epoch: run.envelope.core_epoch,
            hook_sha256: run.envelope.expected_hook_sha256.clone(),
            phase,
            previous: (phase != FrameworkWorkerPhase::Prepared)
                .then(FrameworkObservation::unavailable),
        }
    }
    fn ack(envelope: &FrameworkEnvelope) -> FrameworkAcknowledgement {
        FrameworkAcknowledgement {
            version: vpnhide_diagnostic_model::framework::FRAMEWORK_PROTOCOL,
            session: envelope.session,
            core_epoch: envelope.core_epoch,
            nonce: envelope.nonce.clone(),
            generation: envelope.generation,
            phase: envelope.phase,
            worker_uid: envelope.worker_uid,
            worker_pid: envelope.worker_pid,
            worker_start_ticks: envelope.worker_start_ticks,
            sources: FrameworkSourceIdentity {
                vpn_handle: 151,
                ordinary_handle: 152,
                profile_sha256: "d".repeat(64),
            },
            fixture: envelope.fixture.clone(),
            system_server_pid: 44,
            system_server_start_ticks: 55,
            hook_sha256: envelope.expected_hook_sha256.clone(),
            networks_source: false,
            capabilities_source: false,
            properties_source: false,
            networks_filtered: false,
            capabilities_filtered: false,
            properties_filtered: false,
            legacy: Default::default(),
        }
    }
    #[test]
    fn real_channel_phase_order_lost_reply_retry_and_cleanup_are_bounded_and_exact() {
        let directory = tempfile::tempdir().unwrap();
        let owner = unsafe { libc::geteuid() };
        let store = Arc::new(FrameworkStore::open(directory.path(), owner, owner).unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let store = store.clone();
            let stop = stop.clone();
            let path = directory.path().to_owned();
            std::thread::spawn(move || {
                let end = Instant::now() + Duration::from_secs(5);
                let mut old = None;
                while !stop.load(Ordering::Relaxed) && Instant::now() < end {
                    if let Ok(Some(envelope)) = store.current()
                        && old.as_ref() != Some(&envelope)
                    {
                        use std::io::Write;
                        let mut file = tempfile::NamedTempFile::new_in(&path).unwrap();
                        file.write_all(&serde_json::to_vec(&ack(&envelope)).unwrap())
                            .unwrap();
                        file.persist(path.join(super::super::framework_fixture::ACK_FILE))
                            .unwrap();
                        old = Some(envelope);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
        };
        let mut run = run();
        let expiry = run.envelope.expires_boot_ms;
        assert_eq!(
            run.step_with(request(&run, FrameworkWorkerPhase::Raw), &store, |_| true),
            Err(DiagnosticError::StaleSession)
        );
        for phase in [
            FrameworkWorkerPhase::Prepared,
            FrameworkWorkerPhase::Raw,
            FrameworkWorkerPhase::Protected,
            FrameworkWorkerPhase::Revoked,
            FrameworkWorkerPhase::Finished,
        ] {
            let request = request(&run, phase);
            let result = run.step_with(request.clone(), &store, |_| true).unwrap();
            assert_eq!(run.step_with(request, &store, |_| true).unwrap(), result);
            assert_eq!(run.envelope.expires_boot_ms, expiry);
        }
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();
        assert_eq!(run.phases.len(), 4);
        assert!(store.current().unwrap().is_none());
        assert!(!evaluate_framework(&run.phases, true).controls.is_empty());
        assert_eq!(
            run.step_with(request(&run, FrameworkWorkerPhase::Raw), &store, |_| true),
            Err(DiagnosticError::StaleSession)
        );
    }
    #[test]
    fn admitted_step_after_cleanup_cannot_republish_even_an_initial_prepared_phase() {
        let directory = tempfile::tempdir().unwrap();
        let owner = unsafe { libc::geteuid() };
        let store = FrameworkStore::open(directory.path(), owner, owner).unwrap();
        let mut run = run();
        let admitted = request(&run, FrameworkWorkerPhase::Prepared);
        assert!(run.cleanup_store(&store).unwrap());
        assert_eq!(
            run.step_with(admitted, &store, |_| true),
            Err(DiagnosticError::Expired)
        );
        assert!(store.current().unwrap().is_none());
    }
    #[test]
    fn known_cleanup_retries_after_ack_failure_without_reopening_authorization() {
        let directory = tempfile::tempdir().unwrap();
        let owner = unsafe { libc::geteuid() };
        let store = FrameworkStore::open(directory.path(), owner, owner).unwrap();
        let mut run = run();
        store.publish(None, &run.envelope).unwrap();
        fs::write(
            directory
                .path()
                .join(super::super::framework_fixture::ACK_FILE),
            b"malformed",
        )
        .unwrap();
        assert!(run.cleanup_store(&store).is_err());
        assert!(store.current().unwrap().is_none());
        assert!(run.cleanup_store(&store).unwrap());
        assert_eq!(
            run.step_with(
                request(&run, FrameworkWorkerPhase::Prepared),
                &store,
                |_| true
            ),
            Err(DiagnosticError::Expired)
        );
    }
    #[test]
    fn changed_code_epoch_observation_and_phase_do_not_relabel_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let owner = unsafe { libc::geteuid() };
        let store = FrameworkStore::open(directory.path(), owner, owner).unwrap();
        let mut run = run();
        let mut bad = request(&run, FrameworkWorkerPhase::Prepared);
        bad.core_epoch += 1;
        assert_eq!(
            run.step_with(bad, &store, |_| true),
            Err(DiagnosticError::StaleSession)
        );
        let mut bad = request(&run, FrameworkWorkerPhase::Prepared);
        bad.hook_sha256 = "e".repeat(64);
        assert_eq!(
            run.step_with(bad, &store, |_| true),
            Err(DiagnosticError::StaleSession)
        );
        let mut bad = request(&run, FrameworkWorkerPhase::Prepared);
        bad.previous = Some(FrameworkObservation::unavailable());
        assert_eq!(
            run.step_with(bad, &store, |_| true),
            Err(DiagnosticError::StaleSession)
        );
        assert!(store.current().unwrap().is_none());
    }
}
