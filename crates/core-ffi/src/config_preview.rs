//! Local pure draft adapters. Core repeats admission against its current state;
//! these functions never read a socket, mutate state or grant effect authority.
use crate::{
    CoreCanonicalConfig, CoreConfigSetField, CoreConfigToggle, CoreEdit, CoreError, CoreFieldEdit,
};
use vpnhide_core_rpc::{self as rpc, config_edit};

fn invalid(detail: String) -> CoreError {
    CoreError::InvalidInput { detail }
}
#[uniffi::export]
pub fn preview_canonical_edit(
    config: CoreCanonicalConfig,
    edit: CoreEdit,
) -> Result<CoreCanonicalConfig, CoreError> {
    let edit: rpc::ConfigEdit = edit.into();
    config_edit::validate_edit(&edit).map_err(invalid)?;
    Ok(config_edit::apply_canonical_edit(&config.into(), &edit).into())
}
#[uniffi::export]
pub fn canonical_candidate_valid(config: CoreCanonicalConfig) -> bool {
    config_edit::validate_candidate(&config.into()).is_ok()
}
#[uniffi::export]
pub fn canonical_toggle_value(config: CoreCanonicalConfig, field: CoreConfigToggle) -> bool {
    config_edit::toggle_value(&config.into(), field.into())
}
#[uniffi::export]
pub fn canonical_set_value(config: CoreCanonicalConfig, field: CoreConfigSetField) -> Vec<String> {
    config_edit::set_value(&config.into(), field.into())
        .iter()
        .cloned()
        .collect()
}
#[uniffi::export]
pub fn canonical_edit_paths(edit: CoreEdit) -> Vec<Vec<String>> {
    config_edit::write_set(&edit.into())
        .into_iter()
        .map(|p| p.split('/').map(str::to_owned).collect())
        .collect()
}
#[uniffi::export]
pub fn canonical_field_diff(
    base: CoreCanonicalConfig,
    desired: CoreCanonicalConfig,
) -> Result<Vec<CoreFieldEdit>, CoreError> {
    let edits = config_edit::diff_fields(&base.into(), &desired.into());
    config_edit::validate_edit(&rpc::ConfigEdit::Batch {
        edits: edits.clone(),
    })
    .map_err(invalid)?;
    edits
        .into_iter()
        .map(|edit| match edit {
            rpc::ConfigEdit::Toggle { field, enabled } => Ok(CoreFieldEdit::Toggle {
                field: field.into(),
                enabled,
            }),
            rpc::ConfigEdit::SettingSet { field, values } => Ok(CoreFieldEdit::SettingSet {
                field: field.into(),
                values: values.into_iter().collect(),
            }),
            rpc::ConfigEdit::App { pkg, field, value } => Ok(CoreFieldEdit::App {
                pkg,
                field: field.into(),
                value: value.into(),
            }),
            rpc::ConfigEdit::RemoveApp { pkg } => Ok(CoreFieldEdit::RemoveApp { pkg }),
            _ => Err(invalid(
                "shared config diff produced a non-field edit".into(),
            )),
        })
        .collect()
}
