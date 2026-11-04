//! Live native-backend observation. Autonomous diagnostics owns its fixtures;
//! no foreign VPN presence or app split-tunnel membership is observed here.
use crate::CoreState;
use std::sync::Arc;
use std::time::Duration;

async fn refresh_backend(state: &Arc<CoreState>) {
    use crate::{activation, backend};
    use std::path::Path;
    // Only this source owns live/installed evidence. Boot decision and delivery
    // receipts remain with their owner. A busy KPM read becomes Unknown rather
    // than preserving an old active identity as current forever.
    refresh_zygisk_lifecycle(state).await;
    let before = state.publisher.current().snapshot;
    let ctl = activation::kmod_ctl_path_from_env()
        .unwrap_or_else(|| Path::new("/proc/vpnhide_ctl").to_path_buf());
    let modules = backend::kernel_modules_from_env();
    let kpm = backend::kpm_lock_from_env();
    let observed = tokio::task::spawn_blocking(move || {
        backend::detect_managed(
            &ctl,
            modules
                .as_deref()
                .unwrap_or_else(|| Path::new(backend::MODULES_ROOT)),
            kpm.as_deref(),
        )
    })
    .await;
    let Ok(observed) = observed else {
        return;
    };
    state.publisher.update_if_changed(|snapshot| {
        // A delivery/boot transition after the read started supersedes this sample.
        if snapshot.backend.boot != before.backend.boot
            || snapshot.backend.selected != before.backend.selected
            || snapshot.backend.applied != before.backend.applied
            || snapshot.config_revision != before.config_revision
            || snapshot.apply_evidence != before.apply_evidence
        {
            return false;
        }
        merge_backend_observation(&mut snapshot.backend, observed)
    });
}

async fn refresh_zygisk_lifecycle(state: &Arc<CoreState>) {
    let state = state.clone();
    let _ = tokio::task::spawn_blocking(move || state.refresh_zygisk_lifecycle()).await;
}

fn merge_backend_observation(
    current: &mut vpnhide_core_rpc::NativeBackendStatus,
    observed: vpnhide_core_rpc::NativeBackendStatus,
) -> bool {
    let changed = current.active != observed.active
        || current.installed != observed.installed
        || current.observed_freshness != observed.observed_freshness
        || current.runtime_conflict != observed.runtime_conflict
        || current.runtime_status != observed.runtime_status
        || current.observation_error != observed.observation_error;
    current.active = observed.active;
    current.installed = observed.installed;
    current.observed_freshness = observed.observed_freshness;
    current.runtime_conflict = observed.runtime_conflict;
    current.runtime_status = observed.runtime_status;
    current.observation_error = observed.observation_error;
    changed
}

/// Refresh current backend evidence independently of foreign network state.
pub async fn supervise(state: Arc<CoreState>) {
    loop {
        refresh_backend(&state).await;
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_observation_preserves_boot_and_delivery_evidence() {
        use vpnhide_core_rpc::{
            NativeBackend, NativeBackendStatus, NativeBootOutcome, NativeBootStatus,
            ObservationFreshness, SelectionMode, ZygiskTargets,
        };
        let boot = NativeBootStatus {
            selection_evidence: None,
            selection_conflict: None,
            boot_id: "boot-before-observation".into(),
            decided_epoch: 7,
            decided_at_ms: 1,
            updated_at_ms: 2,
            outcome: NativeBootOutcome::Selected {
                backend: NativeBackend::Kpm,
                mode: SelectionMode::Loaded,
                filesystem_hiding: Some(true),
            },
            installed: vec![NativeBackend::Kpm],
            live: vec![NativeBackend::Kpm],
            staged: vec![],
            kmod: None,
            zygisk_targets: ZygiskTargets::Retired,
            ports: None,
            obsolete_modules: vec![],
        };
        let mut current = NativeBackendStatus {
            selected: Some(NativeBackend::Kpm),
            applied: Some(NativeBackend::Kpm),
            active: Some(NativeBackend::Kpm),
            boot: Some(boot.clone()),
            ..Default::default()
        };
        let observed = NativeBackendStatus {
            installed: vec![NativeBackend::Kpm],
            observed_freshness: ObservationFreshness::Unknown,
            ..Default::default()
        };
        assert!(merge_backend_observation(&mut current, observed.clone()));
        assert_eq!(current.boot, Some(boot));
        assert_eq!(current.selected, Some(NativeBackend::Kpm));
        assert_eq!(current.applied, Some(NativeBackend::Kpm));
        assert_eq!(current.active, None);
        assert_eq!(current.observed_freshness, ObservationFreshness::Unknown);
        assert!(!merge_backend_observation(&mut current, observed));
    }
}
