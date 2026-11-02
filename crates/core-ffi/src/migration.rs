//! Shared pure migration previews; privileged commits independently read and bind
//! their own sources in Core. No preview value authorizes cleanup.
use crate::{CoreCanonicalConfig, CoreError, CoreLegacyImportMode};
use std::collections::{BTreeMap, HashMap};
use vpnhide_migration_policy as policy;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreLegacyAppRoles {
    pub package: String,
    pub java: bool,
    pub native: bool,
    pub app_hiding: bool,
    pub ports: bool,
    pub hidden: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreLegacyCandidate {
    pub roles: Vec<CoreLegacyAppRoles>,
    pub unresolved_observer_uids: u32,
    pub invalid_lines: u32,
    pub source_read_complete: bool,
}
fn roles(v: Vec<CoreLegacyAppRoles>) -> BTreeMap<String, policy::LegacyRoles> {
    v.into_iter()
        .map(|a| {
            (
                a.package,
                policy::LegacyRoles {
                    java: a.java,
                    native: a.native,
                    app_hiding: a.app_hiding,
                    ports: a.ports,
                    hidden: a.hidden,
                },
            )
        })
        .collect()
}
#[uniffi::export]
pub fn preview_legacy_config(
    sections: HashMap<String, String>,
    packages: HashMap<String, Vec<u32>>,
    inventory_complete: bool,
) -> Result<CoreLegacyCandidate, CoreError> {
    if sections.len() > 32
        || sections.values().any(|s| s.len() > 64 * 1024)
        || packages.len() > 32_768
        || packages.iter().any(|(p, u)| p.len() > 256 || u.len() > 256)
    {
        return Err(CoreError::InvalidInput {
            detail: "legacy preview exceeds limits".into(),
        });
    }
    let sources = policy::SourceRole::ALL
        .into_iter()
        .filter_map(|r| sections.get(r.section()).map(|s| (r, s.clone())))
        .collect();
    let parsed = policy::parse(
        &sources,
        &packages.into_iter().collect(),
        inventory_complete,
    );
    Ok(CoreLegacyCandidate {
        roles: parsed
            .roles
            .into_iter()
            .map(|(package, a)| CoreLegacyAppRoles {
                package,
                java: a.java,
                native: a.native,
                app_hiding: a.app_hiding,
                ports: a.ports,
                hidden: a.hidden,
            })
            .collect(),
        unresolved_observer_uids: parsed.unresolved_observer_uids.len() as u32,
        invalid_lines: parsed.invalid_lines,
        source_read_complete: parsed.complete,
    })
}
#[uniffi::export]
pub fn apply_legacy_config_preview(
    base: CoreCanonicalConfig,
    apps: Vec<CoreLegacyAppRoles>,
    mode: CoreLegacyImportMode,
    self_package: String,
) -> CoreCanonicalConfig {
    let candidate = policy::LegacyCandidate {
        roles: roles(apps),
        unresolved_observer_uids: Vec::new(),
        invalid_lines: 0,
        complete: false,
    };
    policy::apply(&base.into(), &candidate, mode.into(), &self_package).into()
}
#[uniffi::export]
pub fn legacy_has_user_apps(base: CoreCanonicalConfig, self_package: String) -> bool {
    policy::has_user_apps(&base.into(), &self_package)
}
