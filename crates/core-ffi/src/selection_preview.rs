//! Typed local adapters for shared role/auto-hide projections, never root IO.
use crate::{CoreAppConfig, CoreCanonicalConfig};
use std::collections::HashMap;
use vpnhide_core_rpc::config_selection::{self as selection, AutoHideSignal, RoleSets};

#[derive(uniffi::Record)]
pub struct CoreRoleSets {
    pub java: Vec<String>,
    pub native: Vec<String>,
    pub hidden: Vec<String>,
    pub observers: Vec<String>,
    pub ports: Vec<String>,
}
impl From<CoreRoleSets> for RoleSets {
    fn from(v: CoreRoleSets) -> Self {
        Self {
            java: v.java.into_iter().collect(),
            native: v.native.into_iter().collect(),
            hidden: v.hidden.into_iter().collect(),
            observers: v.observers.into_iter().collect(),
            ports: v.ports.into_iter().collect(),
        }
    }
}

#[derive(uniffi::Record)]
pub struct CoreAutoHideSignal {
    pub package: String,
    pub declares_vpn_service: bool,
    pub name_contains_vpn: bool,
}
fn signals(values: Vec<CoreAutoHideSignal>) -> Vec<AutoHideSignal> {
    values
        .into_iter()
        .map(|v| AutoHideSignal {
            package: v.package,
            declares_vpn_service: v.declares_vpn_service,
            name_contains_vpn: v.name_contains_vpn,
        })
        .collect()
}

#[uniffi::export]
pub fn preview_role_config(
    debug: bool,
    debug_switch: Option<bool>,
    roles: CoreRoleSets,
    existing: Option<CoreCanonicalConfig>,
) -> CoreCanonicalConfig {
    selection::build_roles(
        debug,
        debug_switch,
        &roles.into(),
        existing.map(Into::into).as_ref(),
    )
    .into()
}

#[uniffi::export]
pub fn preview_hidden_packages(existing: Vec<String>, self_pkg: String) -> Vec<String> {
    selection::hidden_packages(&existing.into_iter().collect(), &self_pkg)
        .into_iter()
        .collect()
}

#[uniffi::export]
pub fn preview_auto_hidden(
    config: CoreCanonicalConfig,
    self_pkg: String,
    observed: Vec<CoreAutoHideSignal>,
    partial: bool,
) -> CoreCanonicalConfig {
    selection::reconcile_auto_hidden(config.into(), &self_pkg, &signals(observed), partial).into()
}

#[uniffi::export]
pub fn preview_manual_hidden(config: CoreCanonicalConfig, self_pkg: String) -> Vec<String> {
    selection::manual_hidden(&config.into(), &self_pkg)
        .into_iter()
        .collect()
}

#[uniffi::export]
pub fn preview_manual_hidden_update(
    config: CoreCanonicalConfig,
    self_pkg: String,
    visible: Vec<String>,
    selected: Vec<String>,
    observed: Vec<CoreAutoHideSignal>,
) -> CoreCanonicalConfig {
    selection::update_manual_hidden(
        config.into(),
        &self_pkg,
        &visible.into_iter().collect(),
        &selected.into_iter().collect(),
        &signals(observed),
    )
    .into()
}

#[uniffi::export]
pub fn preview_picker_save(
    base: CoreCanonicalConfig,
    debug: bool,
    self_pkg: String,
    selections: HashMap<String, CoreAppConfig>,
    observed: Vec<CoreAutoHideSignal>,
    partial: bool,
) -> CoreCanonicalConfig {
    selection::picker_save(
        base.into(),
        debug,
        &self_pkg,
        selections
            .into_iter()
            .map(|(pkg, value)| (pkg, value.into()))
            .collect(),
        &signals(observed),
        partial,
    )
    .into()
}
