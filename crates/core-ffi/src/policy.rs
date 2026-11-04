//! Synchronous local draft preview. Shares Core's projector; never performs RPC or IO.
use crate::{CoreCanonicalConfig, CoreNativeBackend};
use std::collections::HashMap;
use vpnhide_native_policy::{NativeHookFamily, NativeSelection, NativeSelectionDetail};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativePolicyUsage {
    pub target_uids: u64,
    pub target_capacity: u64,
    pub wire_bytes: u64,
    pub wire_capacity: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreNativePickerDecision {
    Allowed,
    InventoryUnavailable,
    Capacity { usage: CoreNativePolicyUsage },
}

/// Check all Core-supplied current/staged families in one local call. Reductions
/// remain possible without inventing a complete inventory or a selected backend.
#[uniffi::export]
pub fn preview_native_picker_change(
    before: CoreCanonicalConfig,
    after: CoreCanonicalConfig,
    backends: Vec<CoreNativeBackend>,
    package_uids: HashMap<String, Vec<u32>>,
    inventory_complete: bool,
) -> CoreNativePickerDecision {
    let (_, before_fs, before_selections) = policy_input(before.clone());
    let (_, after_fs, after_selections) = policy_input(after.clone());
    let families = if backends.is_empty() {
        vec![
            NativeHookFamily::Builtin,
            NativeHookFamily::Kmod,
            NativeHookFamily::Kpm,
            NativeHookFamily::Zygisk,
        ]
    } else {
        backends.iter().cloned().map(family).collect()
    };
    if families.iter().all(|family| {
        vpnhide_native_policy::is_reduction(
            &before_selections,
            before_fs,
            &after_selections,
            after_fs,
            *family,
        )
    }) {
        return CoreNativePickerDecision::Allowed;
    }
    if !inventory_complete || backends.is_empty() {
        return CoreNativePickerDecision::InventoryUnavailable;
    }
    for backend in backends {
        let current = preview_native_policy(before.clone(), backend.clone(), package_uids.clone());
        let candidate = preview_native_policy(after.clone(), backend, package_uids.clone());
        if (candidate.target_uids > candidate.target_capacity
            && candidate.target_uids > current.target_uids)
            || (candidate.wire_bytes > candidate.wire_capacity
                && candidate.wire_bytes > current.wire_bytes)
        {
            return CoreNativePickerDecision::Capacity { usage: candidate };
        }
    }
    CoreNativePickerDecision::Allowed
}

#[uniffi::export]
pub fn preview_native_picker_save(
    config: CoreCanonicalConfig,
    backends: Vec<CoreNativeBackend>,
    package_uids: HashMap<String, Vec<u32>>,
    inventory_complete: bool,
) -> CoreNativePickerDecision {
    if !inventory_complete || backends.is_empty() {
        return CoreNativePickerDecision::InventoryUnavailable;
    }
    for backend in backends {
        let usage = preview_native_policy(config.clone(), backend, package_uids.clone());
        if usage.target_uids > usage.target_capacity || usage.wire_bytes > usage.wire_capacity {
            return CoreNativePickerDecision::Capacity { usage };
        }
    }
    CoreNativePickerDecision::Allowed
}

/// Inventory must be the accepted complete Core observation, including every
/// profile. A partial Android UI icon scan must never replace these UID records.
#[uniffi::export]
pub fn preview_native_policy(
    config: CoreCanonicalConfig,
    backend: CoreNativeBackend,
    package_uids: HashMap<String, Vec<u32>>,
) -> CoreNativePolicyUsage {
    let family = family(backend);
    let (debug, filesystem, selections) = policy_input(config);
    let usage = vpnhide_native_policy::usage(
        debug,
        &selections,
        &package_uids.into_iter().collect(),
        family,
        filesystem,
    );
    CoreNativePolicyUsage {
        target_uids: usage.target_uids as u64,
        target_capacity: usage.limits.target_uids as u64,
        wire_bytes: usage.wire_bytes as u64,
        wire_capacity: usage.limits.wire_bytes as u64,
    }
}

fn family(backend: CoreNativeBackend) -> NativeHookFamily {
    match backend {
        CoreNativeBackend::Kmod => NativeHookFamily::Kmod,
        CoreNativeBackend::Builtin => NativeHookFamily::Builtin,
        CoreNativeBackend::Kpm => NativeHookFamily::Kpm,
        CoreNativeBackend::Zygisk => NativeHookFamily::Zygisk,
    }
}

fn policy_input(
    config: CoreCanonicalConfig,
) -> (
    bool,
    bool,
    std::collections::BTreeMap<String, NativeSelection>,
) {
    let filesystem = config
        .settings
        .optional_features
        .iter()
        .any(|feature| feature == vpnhide_native_policy::FILESYSTEM_FEATURE);
    let selections = config
        .apps
        .into_iter()
        .map(|(package, app)| {
            (
                package,
                NativeSelection::Detailed(NativeSelectionDetail {
                    enabled: app.native.enabled,
                    kernel: app.native.kernel,
                    zygisk: app.native.zygisk,
                }),
            )
        })
        .collect();
    (config.debug, filesystem, selections)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpnhide_core_rpc::{AppConfig, CanonicalConfig, NativeRole};

    fn config() -> CoreCanonicalConfig {
        let mut config = CanonicalConfig::default();
        config.apps.insert(
            "target".into(),
            AppConfig {
                native: NativeRole {
                    enabled: true,
                    kernel: None,
                    zygisk: None,
                },
                ..Default::default()
            },
        );
        config.into()
    }

    #[test]
    fn every_current_and_staged_budget_is_checked_in_either_order() {
        let config = config();
        let cap = preview_native_policy(config.clone(), CoreNativeBackend::Kpm, HashMap::new())
            .target_capacity as u32;
        let inventory = HashMap::from([("target".into(), (2_010_000..2_010_000 + cap).collect())]);
        assert!(matches!(
            preview_native_picker_save(
                config.clone(),
                vec![CoreNativeBackend::Kmod],
                inventory.clone(),
                true
            ),
            CoreNativePickerDecision::Allowed
        ));
        for backends in [
            vec![CoreNativeBackend::Kpm, CoreNativeBackend::Kmod],
            vec![CoreNativeBackend::Kmod, CoreNativeBackend::Kpm],
        ] {
            assert!(matches!(
                preview_native_picker_save(
                    config.clone(),
                    backends.clone(),
                    inventory.clone(),
                    true
                ),
                CoreNativePickerDecision::Capacity { .. }
            ));
            assert!(matches!(
                preview_native_picker_change(
                    CanonicalConfig::default().into(),
                    config.clone(),
                    backends,
                    inventory.clone(),
                    true
                ),
                CoreNativePickerDecision::Capacity { .. }
            ));
        }
    }

    #[test]
    fn overflowing_drafts_can_reduce_without_inventory_but_cannot_grow() {
        let before = config();
        let after: CoreCanonicalConfig = CanonicalConfig::default().into();
        assert_eq!(
            preview_native_picker_change(
                before.clone(),
                after.clone(),
                vec![],
                HashMap::new(),
                false
            ),
            CoreNativePickerDecision::Allowed
        );
        assert_eq!(
            preview_native_picker_change(
                after.clone(),
                before.clone(),
                vec![],
                HashMap::new(),
                true
            ),
            CoreNativePickerDecision::InventoryUnavailable
        );
        assert_eq!(
            preview_native_picker_change(
                after,
                before.clone(),
                vec![CoreNativeBackend::Kmod],
                HashMap::new(),
                false
            ),
            CoreNativePickerDecision::InventoryUnavailable
        );
        assert_eq!(
            preview_native_picker_save(before, vec![], HashMap::new(), true),
            CoreNativePickerDecision::InventoryUnavailable
        );
    }
}
