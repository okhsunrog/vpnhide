//! Explicit warned adoption of the old separate discovery module. Old files
//! are never copied as new code, executed, or taken as hook-health evidence.
use super::*;
use crate::maintenance::legacy_modules::LegacyInventory;
use std::path::Path;
use vpnhide_core_rpc::PhaseOutcome;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyAdoption {
    pub inventory: LegacyInventory,
    pub retirement_intent: bool,
    pub retired: bool,
}
impl LegacyAdoption {
    pub(crate) fn valid(&self, work: &work::ProjectionWork) -> bool {
        valid_inventory(&self.inventory)
            && (!self.retirement_intent || (work.dispatch_intent && work.staging.is_some()))
            && (!self.retired
                || (self.retirement_intent && work.outcome == PhaseOutcome::Confirmed))
            && (work.outcome != PhaseOutcome::Confirmed || self.retired)
            && work.previous_withdrawal.is_none()
    }
    pub(crate) fn permits(&self, next: &Self) -> bool {
        self.inventory == next.inventory
            && (!self.retirement_intent || next.retirement_intent)
            && (!self.retired || next.retired)
    }
    pub(crate) fn intermediate_state(&self, work: &work::ProjectionWork) -> bool {
        self.retirement_intent
            && !self.retired
            && work.dispatch_intent
            && work
                .staging
                .as_ref()
                .is_some_and(|staging| staging.valid_for(&work.descriptor))
            && matches!(
                (work.outcome, work.failure),
                (PhaseOutcome::Running, None)
                    | (
                        PhaseOutcome::Unknown,
                        Some(
                            work::ProjectionFailure::ReadbackUnknown
                                | work::ProjectionFailure::ManagerOutcomeUnknown
                        )
                    )
            )
    }
}
fn valid_inventory(inventory: &LegacyInventory) -> bool {
    !inventory.modules.is_empty()
        && inventory.modules.len() <= 2
        && inventory.modules.iter().all(|module| {
            matches!(
                module.relative_path.as_str(),
                "data/adb/modules/vpnhide_zygisk" | "data/adb/modules_update/vpnhide_zygisk"
            ) && module.root.1 > 0
                && module.files.len() <= 256
                && module.directories.len() <= 256
                && module.files.contains_key("module.prop")
                && module.files.iter().all(|(name, file)| {
                    Path::new(name)
                        .components()
                        .all(|part| matches!(part, std::path::Component::Normal(_)))
                        && file.valid(64 * 1024 * 1024)
                })
        })
        && inventory
            .modules
            .windows(2)
            .all(|pair| pair[0].relative_path < pair[1].relative_path)
}

pub(super) struct PreparedLegacyAdoption {
    pub inventory: LegacyInventory,
    _leases: Vec<File>,
}
impl PreparedLegacyAdoption {
    pub(super) fn capture(root: &Path, uid: u32) -> io::Result<Option<Self>> {
        let inventory = LegacyInventory::capture_owned(root, uid)?;
        if inventory.modules.is_empty() {
            return Ok(None);
        }
        if !valid_inventory(&inventory) {
            return Err(invalid());
        }
        let mut leases = Vec::new();
        for module in &inventory.modules {
            let directory = super::directory(&root.join(&module.relative_path))?;
            if identity(&directory)? != module.root {
                return Err(invalid());
            }
            for name in module.files.keys() {
                leases.push(open_beneath(&directory, name)?);
            }
            leases.push(directory);
        }
        inventory.require_unchanged_owned(root, uid)?;
        Ok(Some(Self {
            inventory,
            _leases: leases,
        }))
    }
    pub(super) fn work(&self) -> LegacyAdoption {
        LegacyAdoption {
            inventory: self.inventory.clone(),
            retirement_intent: false,
            retired: false,
        }
    }
}

impl PreparedProjection {
    pub(super) fn require_legacy_unchanged(&self) -> io::Result<()> {
        if let Some(legacy) = &self.legacy {
            legacy
                .inventory
                .require_unchanged_owned(&self.paths.root, self.uid)
        } else if LegacyInventory::capture_owned(&self.paths.root, self.uid)?
            .modules
            .is_empty()
        {
            Ok(())
        } else {
            Err(invalid())
        }
    }
    pub(crate) fn legacy_context(&self) -> Option<(&Path, &LegacyInventory)> {
        self.legacy
            .as_ref()
            .map(|legacy| (self.paths.root.as_path(), &legacy.inventory))
    }
    pub(super) fn retire_legacy(&self, work: &work::ProjectionWork) -> io::Result<()> {
        let legacy = self.legacy.as_ref().ok_or_else(invalid)?;
        let adoption = work.legacy_adoption.as_ref().ok_or_else(invalid)?;
        if !adoption.retirement_intent
            || adoption.inventory != legacy.inventory
            || work.staging.as_ref() != Some(&self.verify_staged()?)
        {
            return Err(invalid());
        }
        self.manager.require_unchanged()?;
        legacy.inventory.retire_owned(&self.paths.root, self.uid)?;
        if !legacy
            .inventory
            .retirement_observed_owned(&self.paths.root, self.uid)?
        {
            return Err(invalid());
        }
        work.staging
            .as_ref()
            .ok_or_else(invalid)?
            .require_unchanged_owned(&self.paths, &self.descriptor, self.uid)
    }
}

#[cfg(test)]
pub(crate) mod tests;
