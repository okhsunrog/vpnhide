//! Boot-write state machine with injected IO. The command-line tool exposes no
//! device-write adapter; real partition/consent adapters require separate review
//! and device acceptance. Every post-dispatch uncertainty fences re-execution.
use crate::kernel::KernelEvidence;
use crate::{
    ProvisionDecision, ProvisionError, RuntimeEvidence, ToolTuple, assess, digest, sha256,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Slot {
    Single,
    A,
    B,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootFacts {
    pub boot_id: String,
    pub active_slot: Slot,
    pub target_slot: Slot,
    pub partition_device: (u32, u32),
    pub partition_uuid: String,
    pub partition_bytes: u64,
    pub ota_idle: Option<bool>,
    pub snapshot_merge_idle: Option<bool>,
    pub bootloader_unlocked: Option<bool>,
    pub avb_unlocked: Option<bool>,
    pub state_sha256: String,
}
impl BootFacts {
    fn ready(&self) -> Result<(), ProvisionError> {
        if self.boot_id.is_empty()
            || self.partition_uuid.is_empty()
            || self.partition_bytes == 0
            || !digest(&self.state_sha256)
        {
            return Err(ProvisionError::BootFactsUnknown);
        }
        if self.active_slot != self.target_slot {
            return Err(ProvisionError::SlotChanged);
        }
        if self.ota_idle != Some(true) || self.snapshot_merge_idle != Some(true) {
            return Err(ProvisionError::OtaInProgress);
        }
        if self.bootloader_unlocked != Some(true) || self.avb_unlocked != Some(true) {
            return Err(ProvisionError::AvbUnconfirmed);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BootPlan {
    operation: u64,
    facts: BootFacts,
    tools: ToolTuple,
    runtime: RuntimeEvidence,
    original_sha256: String,
    original_bytes: u64,
    patched_sha256: String,
    patched_bytes: u64,
    kernel: KernelEvidence,
}
impl BootPlan {
    pub fn prepare(
        operation: u64,
        facts: BootFacts,
        runtime: &RuntimeEvidence,
        tools: ToolTuple,
        kernel: KernelEvidence,
        original: &[u8],
        patched: &[u8],
    ) -> Result<Self, ProvisionError> {
        facts.ready()?;
        if operation == 0
            || facts.boot_id != runtime.boot_id
            || original.is_empty()
            || patched.is_empty()
            || original.len() as u64 != facts.partition_bytes
            || facts.partition_bytes > 512 * 1024 * 1024
            || patched.len() as u64 > facts.partition_bytes
        {
            return Err(ProvisionError::PlanChanged);
        }
        if assess(Some(runtime), &kernel, &tools) != ProvisionDecision::OfflinePatchEligible {
            return Err(ProvisionError::EngineEvidenceMissing);
        }
        Ok(Self {
            operation,
            facts,
            tools,
            runtime: runtime.clone(),
            original_sha256: sha256(original),
            original_bytes: original.len() as u64,
            patched_sha256: sha256(patched),
            patched_bytes: patched.len() as u64,
            kernel,
        })
    }
    fn validate(&self) -> Result<(), ProvisionError> {
        self.facts.ready()?;
        if self.operation == 0
            || self.runtime.boot_id != self.facts.boot_id
            || self.original_bytes != self.facts.partition_bytes
            || self.original_bytes == 0
            || self.original_bytes > 512 * 1024 * 1024
            || self.patched_bytes == 0
            || self.patched_bytes > self.original_bytes
            || !digest(&self.original_sha256)
            || !digest(&self.patched_sha256)
            || !digest(&self.kernel.image_sha256)
            || !digest(&self.kernel.config_sha256)
            || self.kernel.image_bytes < 64
            || assess(Some(&self.runtime), &self.kernel, &self.tools)
                != ProvisionDecision::OfflinePatchEligible
        {
            return Err(ProvisionError::PlanChanged);
        }
        Ok(())
    }
    pub fn sha256(&self) -> String {
        sha256(
            serde_json::to_vec(self)
                .expect("finite boot plan")
                .as_slice(),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootConsent {
    pub plan_sha256: String,
    pub boot_id: String,
    pub expires_monotonic_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WriteState {
    Prepared,
    Writing,
    Confirmed,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteReceipt {
    pub operation: u64,
    pub plan_sha256: String,
    pub state: WriteState,
    pub backup_sha256: String,
}
/// Implementations are trusted adapters, never caller-selected root paths or
/// command text. Backup is durable outside a replaceable module directory.
pub trait BootIo {
    fn observe(&mut self) -> Result<BootFacts, ProvisionError>;
    fn read_original(&mut self, bytes: u64) -> Result<Vec<u8>, ProvisionError>;
    fn backup_and_sync(
        &mut self,
        operation: u64,
        original: &[u8],
    ) -> Result<String, ProvisionError>;
    fn load_receipt(&mut self) -> Result<Option<WriteReceipt>, ProvisionError>;
    fn sync_receipt(&mut self, receipt: &WriteReceipt) -> Result<(), ProvisionError>;
    fn write_once(&mut self, patched: &[u8]) -> Result<(), ProvisionError>;
    /// Flush and perform an independent device read, not an echo of input bytes.
    fn flush_and_readback(&mut self, bytes: u64) -> Result<Vec<u8>, ProvisionError>;
}
pub fn execute(
    plan: &BootPlan,
    consent: &BootConsent,
    now_ms: u64,
    patched: &[u8],
    io: &mut impl BootIo,
) -> Result<WriteReceipt, ProvisionError> {
    plan.validate()?;
    if consent.plan_sha256 != plan.sha256() || consent.boot_id != plan.facts.boot_id {
        return Err(ProvisionError::ConsentMissing);
    }
    if now_ms >= consent.expires_monotonic_ms {
        return Err(ProvisionError::ConsentExpired);
    }
    if sha256(patched) != plan.patched_sha256 || patched.len() as u64 != plan.patched_bytes {
        return Err(ProvisionError::PlanChanged);
    }
    if let Some(previous) = io.load_receipt()? {
        if previous.plan_sha256 == plan.sha256() && previous.state == WriteState::Confirmed {
            return Ok(previous);
        }
        if previous.operation >= plan.operation || previous.state != WriteState::Confirmed {
            return Err(ProvisionError::HeldUnknown);
        }
    }
    if io.observe()? != plan.facts {
        return Err(ProvisionError::SlotChanged);
    }
    let original = io.read_original(plan.original_bytes)?;
    if original.len() as u64 != plan.original_bytes || sha256(&original) != plan.original_sha256 {
        return Err(ProvisionError::SourceChanged);
    }
    let backup_sha256 = io.backup_and_sync(plan.operation, &original)?;
    if backup_sha256 != plan.original_sha256 {
        return Err(ProvisionError::BackupUnconfirmed);
    }
    if io.observe()? != plan.facts || io.read_original(plan.original_bytes)? != original {
        return Err(ProvisionError::SourceChanged);
    }
    let mut receipt = WriteReceipt {
        operation: plan.operation,
        plan_sha256: plan.sha256(),
        state: WriteState::Writing,
        backup_sha256,
    };
    // Intent must be durable before dispatch. Recovery treats Writing as Unknown.
    io.sync_receipt(&receipt)?;
    // Recheck after potentially slow durable IO, directly before dispatch.
    if io.observe()? != plan.facts || io.read_original(plan.original_bytes)? != original {
        return Err(ProvisionError::SourceChanged);
    }
    let result = io
        .write_once(patched)
        .and_then(|()| io.flush_and_readback(plan.patched_bytes));
    match result {
        Ok(readback)
            if readback.len() as u64 == plan.patched_bytes
                && sha256(&readback) == plan.patched_sha256 =>
        {
            receipt.state = WriteState::Confirmed;
            io.sync_receipt(&receipt)
                .map_err(|_| ProvisionError::WriteUnknown)?;
            Ok(receipt)
        }
        other => {
            receipt.state = WriteState::Unknown;
            let _ = io.sync_receipt(&receipt);
            Err(if other.is_ok() {
                ProvisionError::ReadbackMismatch
            } else {
                ProvisionError::WriteUnknown
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AllDisabledQualification, KernelPatchObservation, RuntimeFamily, ToolIdentity};
    fn plan() -> BootPlan {
        let original = vec![1; 4096];
        let tool = ToolIdentity {
            sha256: sha256(b"tool"),
            bytes: 4,
            source_commit: "a".repeat(40),
        };
        let tools = ToolTuple {
            engine: tool.clone(),
            kptools: tool.clone(),
            kpatch: tool.clone(),
            magiskboot: tool.clone(),
            all_disabled_qualification: Some(AllDisabledQualification {
                engine_sha256: tool.sha256,
                evidence_sha256: sha256(b"qualified"),
            }),
        };
        let runtime = RuntimeEvidence {
            boot_id: "boot".into(),
            family: RuntimeFamily::KernelSu,
            manager_handshake_sha256: sha256(b"actual manager"),
            kernelpatch: KernelPatchObservation::Absent {
                probe_sha256: sha256(b"observed absence"),
            },
        };
        BootPlan::prepare(
            1,
            BootFacts {
                boot_id: "boot".into(),
                active_slot: Slot::A,
                target_slot: Slot::A,
                partition_device: (8, 1),
                partition_uuid: "uuid".into(),
                partition_bytes: original.len() as u64,
                ota_idle: Some(true),
                snapshot_merge_idle: Some(true),
                bootloader_unlocked: Some(true),
                avb_unlocked: Some(true),
                state_sha256: sha256(b"state"),
            },
            &runtime,
            tools,
            crate::kernel::inspect_kernel(&crate::kernel::tests::image(
                "CONFIG_KALLSYMS=y\n# CONFIG_KALLSYMS_ALL is not set\n",
            ))
            .unwrap(),
            &original,
            b"patched",
        )
        .unwrap()
    }
    #[derive(Default)]
    struct Io {
        facts: Option<BootFacts>,
        data: Vec<u8>,
        backup: Vec<u8>,
        receipt: Option<WriteReceipt>,
        writes: u32,
        fail: &'static str,
    }
    impl Io {
        fn new(plan: &BootPlan) -> Self {
            Self {
                facts: Some(plan.facts.clone()),
                data: vec![1; 4096],
                ..Self::default()
            }
        }
    }
    impl BootIo for Io {
        fn observe(&mut self) -> Result<BootFacts, ProvisionError> {
            Ok(self.facts.clone().unwrap())
        }
        fn read_original(&mut self, bytes: u64) -> Result<Vec<u8>, ProvisionError> {
            Ok(self.data[..bytes as usize].to_vec())
        }
        fn backup_and_sync(&mut self, _: u64, data: &[u8]) -> Result<String, ProvisionError> {
            self.backup = data.to_vec();
            Ok(if self.fail == "backup" {
                sha256(b"wrong")
            } else {
                sha256(data)
            })
        }
        fn load_receipt(&mut self) -> Result<Option<WriteReceipt>, ProvisionError> {
            Ok(self.receipt.clone())
        }
        fn sync_receipt(&mut self, receipt: &WriteReceipt) -> Result<(), ProvisionError> {
            if self.fail == "intent" && receipt.state == WriteState::Writing {
                return Err(ProvisionError::JournalUnavailable);
            }
            if self.fail == "completion" && receipt.state == WriteState::Confirmed {
                return Err(ProvisionError::JournalUnavailable);
            }
            self.receipt = Some(receipt.clone());
            if self.fail == "slot" {
                self.facts.as_mut().unwrap().active_slot = Slot::B;
            }
            if self.fail == "ota" {
                self.facts.as_mut().unwrap().ota_idle = Some(false);
            }
            Ok(())
        }
        fn write_once(&mut self, data: &[u8]) -> Result<(), ProvisionError> {
            self.writes += 1;
            if self.fail == "write" {
                self.data[0] = 9;
                return Err(ProvisionError::WriteUnknown);
            }
            self.data[..data.len()].copy_from_slice(data);
            Ok(())
        }
        fn flush_and_readback(&mut self, bytes: u64) -> Result<Vec<u8>, ProvisionError> {
            if self.fail == "readback" {
                return Err(ProvisionError::WriteUnknown);
            }
            let mut data = self.data[..bytes as usize].to_vec();
            if self.fail == "short" {
                data.pop();
            }
            if self.fail == "hash" {
                data[0] ^= 1;
            }
            Ok(data)
        }
    }
    fn consent(plan: &BootPlan) -> BootConsent {
        BootConsent {
            plan_sha256: plan.sha256(),
            boot_id: plan.facts.boot_id.clone(),
            expires_monotonic_ms: 100,
        }
    }
    #[test]
    fn confirmed_write_is_deduplicated_and_full_original_is_retained() {
        let plan = plan();
        let mut io = Io::new(&plan);
        let receipt = execute(&plan, &consent(&plan), 0, b"patched", &mut io).unwrap();
        assert_eq!(receipt.state, WriteState::Confirmed);
        assert_eq!(io.backup, vec![1; 4096]);
        assert_eq!(
            execute(&plan, &consent(&plan), 0, b"patched", &mut io).unwrap(),
            receipt
        );
        assert_eq!(io.writes, 1);
    }
    #[test]
    fn source_backup_slot_ota_and_intent_failures_prevent_dispatch() {
        let plan = plan();
        for failure in ["backup", "slot", "ota", "intent"] {
            let mut io = Io::new(&plan);
            io.fail = failure;
            assert!(
                execute(&plan, &consent(&plan), 0, b"patched", &mut io).is_err(),
                "{failure}"
            );
            assert_eq!(io.writes, 0, "{failure}");
        }
        let mut io = Io::new(&plan);
        io.data[0] = 2;
        assert_eq!(
            execute(&plan, &consent(&plan), 0, b"patched", &mut io),
            Err(ProvisionError::SourceChanged)
        );
        assert!(io.backup.is_empty());
    }
    #[test]
    fn post_dispatch_failure_is_held_across_recovery_and_never_replayed() {
        let plan = plan();
        for failure in ["write", "readback", "short", "hash", "completion"] {
            let mut io = Io::new(&plan);
            io.fail = failure;
            assert!(
                execute(&plan, &consent(&plan), 0, b"patched", &mut io).is_err(),
                "{failure}"
            );
            io.fail = "";
            assert_eq!(
                execute(&plan, &consent(&plan), 0, b"patched", &mut io),
                Err(ProvisionError::HeldUnknown),
                "{failure}"
            );
            assert_eq!(io.writes, 1);
            assert_eq!(io.backup, vec![1; 4096]);
        }
    }
    #[test]
    fn mutated_plan_reused_id_or_expired_consent_cannot_write() {
        let original = plan();
        for mutate in [0, 1, 2, 3, 4] {
            let mut plan = original.clone();
            match mutate {
                0 => plan.operation = 0,
                1 => plan.original_bytes = 0,
                2 => plan.patched_bytes = 5000,
                3 => plan.tools.engine.sha256 = sha256(b"unqualified replacement"),
                _ => plan.runtime.kernelpatch = KernelPatchObservation::Unknown,
            }
            let mut io = Io::new(&original);
            assert!(execute(&plan, &consent(&plan), 0, b"patched", &mut io).is_err());
            assert_eq!(io.writes, 0);
            assert!(io.backup.is_empty());
        }
        let mut io = Io::new(&original);
        execute(&original, &consent(&original), 0, b"patched", &mut io).unwrap();
        let mut changed = original.clone();
        changed.facts.state_sha256 = sha256(b"changed");
        assert_eq!(
            execute(&changed, &consent(&changed), 0, b"patched", &mut io),
            Err(ProvisionError::HeldUnknown)
        );
        assert_eq!(io.backup, vec![1; 4096]);
        assert_eq!(io.writes, 1);
        assert_eq!(
            execute(&original, &consent(&original), 100, b"patched", &mut io),
            Err(ProvisionError::ConsentExpired)
        );
    }
    #[test]
    fn unknown_runtime_and_engine_changed_under_all_n_evidence_are_blocked() {
        let plan = plan();
        let mut runtime = plan.runtime.clone();
        runtime.kernelpatch = KernelPatchObservation::Unknown;
        assert_eq!(
            assess(Some(&runtime), &plan.kernel, &plan.tools),
            ProvisionDecision::Blocked(ProvisionError::RuntimeUnknown)
        );
        let mut tools = plan.tools.clone();
        tools.engine.sha256 = sha256(b"other engine");
        assert_eq!(
            assess(Some(&plan.runtime), &plan.kernel, &tools),
            ProvisionDecision::Blocked(ProvisionError::EngineEvidenceMissing)
        );
        assert!(
            BootPlan::prepare(
                2,
                plan.facts.clone(),
                &plan.runtime,
                plan.tools.clone(),
                plan.kernel.clone(),
                b"short",
                b"longer"
            )
            .is_err()
        );
    }
}
