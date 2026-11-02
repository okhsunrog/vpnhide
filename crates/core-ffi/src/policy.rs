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

#[uniffi::export]
pub fn native_policy_is_reduction(
    before: CoreCanonicalConfig,
    after: CoreCanonicalConfig,
    backend: Option<CoreNativeBackend>,
) -> bool {
    let (_, before_fs, before) = policy_input(before);
    let (_, after_fs, after) = policy_input(after);
    let families = backend
        .map(|backend| vec![family(backend)])
        .unwrap_or_else(|| {
            vec![
                NativeHookFamily::Builtin,
                NativeHookFamily::Kmod,
                NativeHookFamily::Kpm,
                NativeHookFamily::Zygisk,
            ]
        });
    families.into_iter().all(|family| {
        vpnhide_native_policy::is_reduction(&before, before_fs, &after, after_fs, family)
    })
}
