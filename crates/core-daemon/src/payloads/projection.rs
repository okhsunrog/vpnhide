//! One canonical policy and one PackageManager snapshot, projected independently
//! for each implementation. This retains the existing semantic family mapping;
//! comparing libc hook IDs to kernel hook IDs would reject a valid alternative.
use vpnhide_activator::PackageUidMap;
use vpnhide_core_rpc::{CanonicalConfig, NativeBackend};
use vpnhide_protocol::hook_ids::Hook;

use super::selection::Policy;
use crate::activation::{ProjectionError, project_native_wire_with_resolver};

#[derive(Debug, Clone)]
pub struct Projection {
    pub wire: String,
    pub requirements: Policy,
}

pub fn project(
    config: &CanonicalConfig,
    backend: NativeBackend,
    packages: &PackageUidMap,
) -> Result<Projection, ProjectionError> {
    let wire = project_native_wire_with_resolver(config, backend, packages)?;
    let parsed = vpnhide_protocol::parse_config(wire.as_bytes())
        .ok_or_else(|| ProjectionError::Failed("invalid projected native policy".into()))?;
    let mut required_hooks = parsed
        .targets
        .iter()
        .fold(parsed.default_mask, |mask, target| mask | target.hookmask);
    // For ko/KPM the existing projection retains the filesystem bit while the
    // loader parameter gates it. It is not required coverage when explicitly off.
    // Do not alter the wire, selected per-app masks or any other default bit.
    if matches!(backend, NativeBackend::Kmod | NativeBackend::Kpm)
        && !config
            .settings
            .optional_features
            .contains(crate::config_edit::FILESYSTEM_FEATURE)
    {
        required_hooks &= !Hook::FilesystemIfacePaths.bit();
    }
    let requirements = Policy {
        resolved: true,
        targets: parsed.targets.len() as u32,
        wire_bytes: wire.len() as u32,
        default_mask: parsed.default_mask,
        required_hooks,
    };
    Ok(Projection { wire, requirements })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpnhide_core_rpc::{AppConfig, NativeRole};

    fn policy() -> (CanonicalConfig, PackageUidMap) {
        let mut config = CanonicalConfig::default();
        config.apps.insert(
            "com.example".into(),
            AppConfig {
                native: NativeRole {
                    enabled: true,
                    kernel: None,
                    zygisk: None,
                },
                ..Default::default()
            },
        );
        (
            config,
            vpnhide_activator::parse_pm_packages("package:com.example uid:10123,110123\n"),
        )
    }

    #[test]
    fn families_use_the_same_profile_inventory_but_distinct_hook_requirements() {
        let (config, packages) = policy();
        let kernel = project(&config, NativeBackend::Builtin, &packages).unwrap();
        let libc = project(&config, NativeBackend::Zygisk, &packages).unwrap();
        let uids = |p: &Projection| {
            vpnhide_protocol::parse_config(p.wire.as_bytes())
                .unwrap()
                .targets
                .into_iter()
                .map(|t| t.uid)
                .collect::<Vec<_>>()
        };
        assert_eq!(uids(&kernel), vec![10123, 110123]);
        assert_eq!(uids(&kernel), uids(&libc));
        assert_ne!(kernel.requirements.required_hooks & Hook::DevIoctl.bit(), 0);
        assert_eq!(libc.requirements.required_hooks & Hook::DevIoctl.bit(), 0);
        assert_ne!(
            libc.requirements.required_hooks & Hook::ZygiskIoctl.bit(),
            0
        );
        assert_eq!(kernel.requirements.default_mask, 0);
    }

    #[test]
    fn load_time_filesystem_gate_is_not_mistaken_for_missing_required_coverage() {
        let (mut config, packages) = policy();
        let disabled = project(&config, NativeBackend::Kmod, &packages).unwrap();
        assert_eq!(
            disabled.requirements.required_hooks & Hook::FilesystemIfacePaths.bit(),
            0
        );
        assert_ne!(
            vpnhide_protocol::parse_config(disabled.wire.as_bytes())
                .unwrap()
                .targets[0]
                .hookmask
                & Hook::FilesystemIfacePaths.bit(),
            0
        );
        config
            .settings
            .optional_features
            .insert(crate::config_edit::FILESYSTEM_FEATURE.into());
        let enabled = project(&config, NativeBackend::Kmod, &packages).unwrap();
        assert_ne!(
            enabled.requirements.required_hooks & Hook::FilesystemIfacePaths.bit(),
            0
        );
    }
}
