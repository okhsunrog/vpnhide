//! Boot-time policy apply without the APK (core-2.0 step 5).
//!
//! After journal fence/reconcile on load, boot re-applies Native/Ports from the
//! durable canonical config when production env enables those writers. Missing
//! config and Held Unknown ops are **explicit recovery conditions**, not silent
//! success.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use vpnhide_core_rpc::{BaseRef, ConfigEdit, OperationId, PhaseOutcome, SubmitError};

use crate::server::CoreState;

/// Reserved high bit so boot ops do not collide with low app-issued ids in tests.
const BOOT_OP_TAG: u64 = 1 << 63;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootApplyStatus {
    /// No durable config — nothing to apply; diagnosable, not an error crash.
    NoIntent,
    /// Prior Unknown still Held — refuse blind re-dispatch.
    HeldUnknown,
    /// RepairActivation admitted and settled Confirmed.
    Applied,
    /// Admitted but settled FailedKnown / Unknown / incomplete.
    AppliedWithFailure { settled: PhaseOutcome },
    /// Admission refused (e.g. Unavailable with no config — also NoIntent path).
    Refused { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootApplyReport {
    pub status: BootApplyStatus,
    pub op: Option<OperationId>,
    pub detail: String,
}

impl BootApplyReport {
    pub fn is_applied_ok(&self) -> bool {
        matches!(self.status, BootApplyStatus::Applied)
    }

    /// Write a small status file for `logcat`/module diagnostics.
    pub fn write_status_file(&self, path: &Path) {
        let body = format!(
            "status={:?}\nop={:?}\ndetail={}\n",
            self.status,
            self.op.map(|o| o.0),
            self.detail.replace('\n', " ")
        );
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, body);
    }
}

fn boot_operation_id() -> OperationId {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    OperationId(BOOT_OP_TAG | (nanos & !BOOT_OP_TAG))
}

/// Re-apply runtime policy from durable config (Native/Ports per env). Does not
/// rewrite canonical JSON. Safe to call once per boot before or after serve.
pub fn reapply(state: &CoreState) -> BootApplyReport {
    if state.has_unknown_hold() {
        return BootApplyReport {
            status: BootApplyStatus::HeldUnknown,
            op: None,
            detail: "journal or in-memory Unknown still Held; refusing boot re-apply".into(),
        };
    }
    if !state.has_config() {
        return BootApplyReport {
            status: BootApplyStatus::NoIntent,
            op: None,
            detail: "no durable canonical config; boot apply skipped".into(),
        };
    }
    let revision = state.config_revision();
    let op = boot_operation_id();
    match state.submit_edit_public(op, BaseRef { revision }, &ConfigEdit::RepairActivation) {
        Ok(_) => {
            let record = state.get_operation_record(op);
            let settled = record
                .as_ref()
                .and_then(|r| r.settled)
                .unwrap_or(PhaseOutcome::Unknown);
            let phases = record
                .as_ref()
                .map(|r| {
                    r.phases
                        .iter()
                        .map(|p| format!("{:?}={:?}", p.phase, p.outcome))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            if settled == PhaseOutcome::Confirmed {
                BootApplyReport {
                    status: BootApplyStatus::Applied,
                    op: Some(op),
                    detail: format!("repair confirmed phases=[{phases}]"),
                }
            } else {
                BootApplyReport {
                    status: BootApplyStatus::AppliedWithFailure { settled },
                    op: Some(op),
                    detail: format!("repair settled={settled:?} phases=[{phases}]"),
                }
            }
        }
        Err(SubmitError::Unavailable) => BootApplyReport {
            status: BootApplyStatus::NoIntent,
            op: None,
            detail: "RepairActivation unavailable (no config)".into(),
        },
        Err(e) => BootApplyReport {
            status: BootApplyStatus::Refused {
                reason: format!("{e:?}"),
            },
            op: None,
            detail: format!("boot repair refused: {e:?}"),
        },
    }
}

/// True when `VPNHIDE_BOOT_APPLY` is truthy (module service.sh sets this).
pub fn boot_apply_enabled() -> bool {
    match std::env::var("VPNHIDE_BOOT_APPLY") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

pub fn default_boot_status_path() -> std::path::PathBuf {
    crate::ownership::owner_dir().join("boot_status")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publisher::Publisher;
    use std::sync::Mutex;
    use vpnhide_core_rpc::ConfigPhase;
    use vpnhide_core_rpc::{CanonicalConfig, StateSnapshot};

    static ENV: Mutex<()> = Mutex::new(());

    #[test]
    fn no_config_is_explicit_no_intent() {
        let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
        let report = reapply(&state);
        assert_eq!(report.status, BootApplyStatus::NoIntent);
    }

    #[test]
    fn repair_with_config_and_no_writers_confirms_noop_plan() {
        // PersistOnly repair path when no ctl and ports off — still Confirmed.
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("cfg.json");
        crate::config_file::write_atomic(&cfg, &CanonicalConfig::default()).unwrap();
        let state =
            CoreState::load(Publisher::new(1, StateSnapshot::default()), cfg, None).unwrap();
        let report = reapply(&state);
        assert!(
            matches!(report.status, BootApplyStatus::Applied),
            "{report:?}"
        );
        let record = state.get_operation_record(report.op.unwrap()).unwrap();
        assert!(
            record
                .phases
                .iter()
                .any(|p| p.phase == ConfigPhase::Persist)
                || record.phases.iter().all(|p| {
                    // NativeOnly/PortsOnly/PersistOnly variants
                    matches!(
                        p.phase,
                        ConfigPhase::Persist | ConfigPhase::Native | ConfigPhase::Ports
                    )
                })
        );
    }

    #[test]
    fn boot_apply_env_gate() {
        let _g = ENV.lock().unwrap();
        unsafe { std::env::remove_var("VPNHIDE_BOOT_APPLY") };
        assert!(!boot_apply_enabled());
        unsafe { std::env::set_var("VPNHIDE_BOOT_APPLY", "1") };
        assert!(boot_apply_enabled());
        unsafe { std::env::remove_var("VPNHIDE_BOOT_APPLY") };
    }
}
