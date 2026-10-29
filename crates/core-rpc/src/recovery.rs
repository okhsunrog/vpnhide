//! One recovery policy shared by core admission and the typed app projection.
use crate::{ConfigPhase, StateSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    Repair,
    SetSuperkey,
    ClearSuperkey,
    Cleanup,
    FullReset,
}

/// Persist uncertainty must be read back before changing a credential setting.
/// These are new explicit recovery commands, never replay of an old operation.
pub fn permits_held(action: RecoveryAction, held: &[ConfigPhase]) -> bool {
    !matches!(
        action,
        RecoveryAction::SetSuperkey | RecoveryAction::ClearSuperkey
    ) || !held.contains(&ConfigPhase::Persist)
}

/// Automatic boot repair may restore only independent runtime resources. Unlike
/// explicit user repair, it must not resolve/retry a held effect. Native delivery
/// also depends on credentials; every delivery depends on known canonical intent.
pub fn permits_boot_phase(phase: ConfigPhase, held: &[ConfigPhase]) -> bool {
    if held.contains(&ConfigPhase::Persist) || held.contains(&phase) {
        return false;
    }
    match phase {
        ConfigPhase::Native => !held.contains(&ConfigPhase::Secret),
        ConfigPhase::Ports => true,
        // Persist is a no-op in a repair without runtime writers.
        ConfigPhase::Persist => true,
        ConfigPhase::Secret | ConfigPhase::Cleanup => false,
    }
}

pub fn refusal(state: &StateSnapshot, action: RecoveryAction) -> Option<&'static str> {
    if state.storage_fault.is_some() {
        return Some("storage_fault_restart_required");
    }
    if state.startup_error.is_some() {
        return Some("startup_fault");
    }
    if !state.operations_in_flight.is_empty() {
        return Some("operation_in_flight");
    }
    if !permits_held(action, &state.held_resources) {
        return Some("persist_uncertain");
    }
    if state.config.is_none()
        && (matches!(
            action,
            RecoveryAction::SetSuperkey | RecoveryAction::ClearSuperkey
        ) || (action == RecoveryAction::Repair
            && !state.held_resources.contains(&ConfigPhase::Persist)))
    {
        return Some("config_missing");
    }
    if action == RecoveryAction::FullReset && (state.debug_lease || !state.debug_leases.is_empty())
    {
        return Some("capture_active");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_hold_combination_keeps_independent_recovery_available() {
        let phases = [
            ConfigPhase::Persist,
            ConfigPhase::Secret,
            ConfigPhase::Cleanup,
            ConfigPhase::Native,
            ConfigPhase::Ports,
        ];
        for bits in 0..32 {
            let state = StateSnapshot {
                config: Some(Default::default()),
                mutation_held: bits != 0,
                held_resources: phases
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| bits & (1 << i) != 0)
                    .map(|(_, p)| *p)
                    .collect(),
                ..Default::default()
            };
            for action in [
                RecoveryAction::Repair,
                RecoveryAction::Cleanup,
                RecoveryAction::FullReset,
            ] {
                assert_eq!(refusal(&state, action), None);
            }
            for action in [RecoveryAction::SetSuperkey, RecoveryAction::ClearSuperkey] {
                assert_eq!(refusal(&state, action).is_none(), bits & 1 == 0);
            }
        }
    }
}
