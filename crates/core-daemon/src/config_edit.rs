//! The pure config-edit reducer — core's config-operation logic in Rust (§7: a reducer
//! whose state core owns runs in core). Ported from the app's `applyCanonicalEdit`
//! (CanonicalEditData.kt) and pinned by the same behavioural cases, the language-neutral
//! vector both implementations must agree on.
//!
//! `apply_canonical_edit(config, edit) -> config` is a pure function: no I/O, no clock, no
//! randomness. Core applies each accepted edit against its *fresh* config (§5), so unrelated
//! fields merge naturally — an edit only rewrites the field it names.

use vpnhide_core_rpc::{
    AppConfig, CanonicalConfig, ConfigAppField, ConfigEdit, ConfigSetField, ConfigToggle,
};

/// The optional-feature key the filesystem-hiding toggle maps to (mirrors
/// `OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS`; the `filesystem_iface_paths` hook name).
pub const FILESYSTEM_FEATURE: &str = "filesystem_iface_paths";

/// An app with no role at all is dropped from `apps` (mirrors `CanonicalApp.hasAnyRole`).
pub fn has_any_role(app: &AppConfig) -> bool {
    app.java || app.native.enabled || app.app_hiding || app.ports || app.hidden
}

/// Apply one typed edit to a config, returning the new config. Pure.
pub fn apply_canonical_edit(config: &CanonicalConfig, edit: &ConfigEdit) -> CanonicalConfig {
    let mut next = config.clone();
    match edit {
        ConfigEdit::Batch { edits } => {
            for edit in edits {
                next = apply_canonical_edit(&next, edit);
            }
        }
        ConfigEdit::Toggle { field, enabled } => apply_toggle(&mut next, *field, *enabled),
        ConfigEdit::SettingSet { field, values } => match field {
            ConfigSetField::OptionalFeatures => next.settings.optional_features = values.clone(),
            ConfigSetField::AutoHideExcludedPackages => {
                next.settings.auto_hide_excluded_packages = values.clone();
            }
            ConfigSetField::AutoHiddenPackages => {
                next.settings.auto_hidden_packages = values.clone();
            }
        },
        ConfigEdit::App { pkg, field, value } => {
            let base = next.apps.get(pkg).cloned().unwrap_or_default();
            let merged = merge_app(&base, value, *field);
            if has_any_role(&merged) {
                next.apps.insert(pkg.clone(), merged);
            } else {
                next.apps.remove(pkg);
            }
        }
        ConfigEdit::RemoveApp { pkg } => {
            next.apps.remove(pkg);
        }
        ConfigEdit::Replace { config } => next = config.clone(),
        ConfigEdit::RepairActivation => {}
    }
    next
}

fn apply_toggle(config: &mut CanonicalConfig, field: ConfigToggle, enabled: bool) {
    match field {
        ConfigToggle::Filesystem => {
            if enabled {
                config
                    .settings
                    .optional_features
                    .insert(FILESYSTEM_FEATURE.to_owned());
            } else {
                config.settings.optional_features.remove(FILESYSTEM_FEATURE);
            }
        }
        ConfigToggle::Debug => config.debug = enabled,
        ConfigToggle::DebugSwitch => config.debug_switch = enabled,
        ConfigToggle::RememberSuperkey => config.settings.remember_superkey = enabled,
        ConfigToggle::AutoHideServices => config.settings.auto_hide_vpn_services = enabled,
        ConfigToggle::AutoHideName => config.settings.auto_hide_vpn_name = enabled,
    }
}

/// Merge one field of `next` into `base` (mirrors `mergeCanonicalApp`).
fn merge_app(base: &AppConfig, next: &AppConfig, field: ConfigAppField) -> AppConfig {
    let mut app = base.clone();
    match field {
        ConfigAppField::Java => {
            app.java = next.java;
            app.java_hooks = next.java_hooks.clone();
        }
        ConfigAppField::Native => app.native = next.native.clone(),
        ConfigAppField::AppHiding => app.app_hiding = next.app_hiding,
        ConfigAppField::Ports => {
            app.ports = next.ports;
            app.port_policy = next.port_policy.clone();
        }
        ConfigAppField::Hidden => app.hidden = next.hidden,
    }
    app
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use vpnhide_core_rpc::{NativeRole, Settings};

    fn empty() -> CanonicalConfig {
        CanonicalConfig::default()
    }

    fn app_with(native: bool) -> AppConfig {
        AppConfig {
            native: NativeRole {
                enabled: native,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn toggles_set_their_field() {
        let mut c = empty();
        c = apply_canonical_edit(
            &c,
            &ConfigEdit::Toggle {
                field: ConfigToggle::Debug,
                enabled: true,
            },
        );
        assert!(c.debug);
        c = apply_canonical_edit(
            &c,
            &ConfigEdit::Toggle {
                field: ConfigToggle::AutoHideName,
                enabled: true,
            },
        );
        assert!(c.settings.auto_hide_vpn_name);
    }

    #[test]
    fn filesystem_toggle_adds_and_removes_the_feature() {
        let on = apply_canonical_edit(
            &empty(),
            &ConfigEdit::Toggle {
                field: ConfigToggle::Filesystem,
                enabled: true,
            },
        );
        assert!(on.settings.optional_features.contains(FILESYSTEM_FEATURE));
        let off = apply_canonical_edit(
            &on,
            &ConfigEdit::Toggle {
                field: ConfigToggle::Filesystem,
                enabled: false,
            },
        );
        assert!(!off.settings.optional_features.contains(FILESYSTEM_FEATURE));
    }

    #[test]
    fn setting_set_replaces_the_whole_set() {
        let c = apply_canonical_edit(
            &empty(),
            &ConfigEdit::SettingSet {
                field: ConfigSetField::AutoHideExcludedPackages,
                values: BTreeSet::from(["com.bank".to_owned()]),
            },
        );
        assert_eq!(
            c.settings.auto_hide_excluded_packages,
            BTreeSet::from(["com.bank".to_owned()])
        );
    }

    #[test]
    fn app_edit_merges_only_the_named_field() {
        // Start with an app that hides; edit only its Native field.
        let mut c = empty();
        c.apps.insert(
            "com.x".into(),
            AppConfig {
                hidden: true,
                ..Default::default()
            },
        );
        let edited = apply_canonical_edit(
            &c,
            &ConfigEdit::App {
                pkg: "com.x".into(),
                field: ConfigAppField::Native,
                value: app_with(true),
            },
        );
        let app = &edited.apps["com.x"];
        assert!(app.native.enabled, "native field applied");
        assert!(app.hidden, "hidden preserved — only Native was merged");
    }

    #[test]
    fn an_app_left_with_no_role_is_dropped() {
        let mut c = empty();
        c.apps.insert("com.x".into(), app_with(true)); // only native
        // Turn native off via an App edit: the app now has no role -> removed.
        let edited = apply_canonical_edit(
            &c,
            &ConfigEdit::App {
                pkg: "com.x".into(),
                field: ConfigAppField::Native,
                value: app_with(false),
            },
        );
        assert!(!edited.apps.contains_key("com.x"));
    }

    #[test]
    fn remove_app_drops_it() {
        let mut c = empty();
        c.apps.insert("com.x".into(), app_with(true));
        let edited = apply_canonical_edit(
            &c,
            &ConfigEdit::RemoveApp {
                pkg: "com.x".into(),
            },
        );
        assert!(edited.apps.is_empty());
    }

    #[test]
    fn replace_swaps_the_whole_config() {
        let replacement = CanonicalConfig {
            debug: true,
            settings: Settings {
                auto_hide_vpn_name: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let edited = apply_canonical_edit(
            &empty(),
            &ConfigEdit::Replace {
                config: replacement.clone(),
            },
        );
        assert_eq!(edited, replacement);
    }

    #[test]
    fn applying_against_fresh_merges_unrelated_fields() {
        // Two edits to different fields compose without clobbering each other.
        let a = apply_canonical_edit(
            &empty(),
            &ConfigEdit::Toggle {
                field: ConfigToggle::Debug,
                enabled: true,
            },
        );
        let b = apply_canonical_edit(
            &a,
            &ConfigEdit::Toggle {
                field: ConfigToggle::RememberSuperkey,
                enabled: true,
            },
        );
        assert!(b.debug && b.settings.remember_superkey);
    }
}

/// A config field path for conflict detection (mirrors Kotlin `ConfigField` segments).
/// Examples: `debug`, `settings/autoHideVpnName`, `apps/com.x/native`, `apps/com.x`.
pub type FieldPath = String;

/// Whether two field paths overlap (prefix relationship), matching `configFieldsOverlap`.
pub fn fields_overlap(a: &str, b: &str) -> bool {
    let a: Vec<&str> = a.split('/').filter(|s| !s.is_empty()).collect();
    let b: Vec<&str> = b.split('/').filter(|s| !s.is_empty()).collect();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    (a.len() >= b.len() && a[..b.len()] == b[..]) || (b.len() >= a.len() && b[..a.len()] == a[..])
}

/// The write set of an edit — the fields it intends to change (§5).
pub fn write_set(edit: &ConfigEdit) -> Vec<FieldPath> {
    match edit {
        ConfigEdit::Batch { edits } => edits
            .iter()
            .flat_map(write_set)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        ConfigEdit::Toggle { field, .. } => vec![toggle_path(*field)],
        ConfigEdit::SettingSet { field, .. } => vec![set_field_path(*field)],
        ConfigEdit::App { pkg, field, .. } => {
            vec![format!("apps/{pkg}/{}", app_field_segment(*field))]
        }
        ConfigEdit::RemoveApp { pkg } => vec![format!("apps/{pkg}")],
        // Import / reset: broad domains — any change under these since base conflicts.
        ConfigEdit::Replace { .. } => vec![
            "apps".into(),
            "settings".into(),
            "debug".into(),
            "debugSwitch".into(),
        ],
        // Activation repair does not rewrite canonical fields.
        ConfigEdit::RepairActivation => Vec::new(),
    }
}

/// A batch is one bounded, atomic field-intent operation. Imports and activation repair
/// are separate commands; nesting cannot hide a broad replacement or unbounded work.
pub fn validate_edit(edit: &ConfigEdit) -> Result<(), String> {
    let ConfigEdit::Batch { edits } = edit else {
        return Ok(());
    };
    if edits.len() > vpnhide_core_rpc::MAX_BATCH_EDITS {
        return Err("too many config edits".into());
    }
    if edits.iter().any(|edit| {
        matches!(
            edit,
            ConfigEdit::Batch { .. } | ConfigEdit::Replace { .. } | ConfigEdit::RepairActivation
        )
    }) {
        return Err("config batch accepts field edits only".into());
    }
    Ok(())
}

/// The daemon validates the resulting canonical value, including clients that talk
/// directly to RPC. A typed FFI request must not bypass the shared disk-codec contract.
pub fn validate_candidate(candidate: &CanonicalConfig) -> Result<(), String> {
    if candidate.version != 1 {
        return Err("unsupported canonical config version".into());
    }
    let json = crate::config_file::to_json_string(candidate);
    if json.len() > 512 * 1024 {
        return Err("canonical config exceeds 512 KiB".into());
    }
    match crate::config_file::parse_status(&json) {
        Ok(crate::config_file::ConfigLoad::Present(parsed)) if parsed == *candidate => Ok(()),
        _ => Err("config does not round-trip through the canonical codec".into()),
    }
}

fn toggle_path(field: ConfigToggle) -> FieldPath {
    match field {
        ConfigToggle::Filesystem => "settings/optionalFeatures".into(),
        ConfigToggle::Debug => "debug".into(),
        ConfigToggle::DebugSwitch => "debugSwitch".into(),
        ConfigToggle::RememberSuperkey => "settings/rememberSuperkey".into(),
        ConfigToggle::AutoHideServices => "settings/autoHideVpnServices".into(),
        ConfigToggle::AutoHideName => "settings/autoHideVpnName".into(),
    }
}

fn set_field_path(field: ConfigSetField) -> FieldPath {
    match field {
        ConfigSetField::OptionalFeatures => "settings/optionalFeatures".into(),
        ConfigSetField::AutoHideExcludedPackages => "settings/autoHideExcludedPackages".into(),
        ConfigSetField::AutoHiddenPackages => "settings/autoHiddenPackages".into(),
    }
}

fn app_field_segment(field: ConfigAppField) -> &'static str {
    match field {
        ConfigAppField::Java => "java",
        ConfigAppField::Native => "native",
        ConfigAppField::AppHiding => "appHiding",
        ConfigAppField::Ports => "ports",
        ConfigAppField::Hidden => "hidden",
    }
}

/// Field-level optimistic concurrency against a map of last-writer revisions.
///
/// Unrelated fields written after `base.revision` do not conflict; a write to the
/// same field (or an overlapping path) after `base` is `Conflict`. Import/reset
/// (`Replace`) uses broad domains so any overlapping change since base conflicts.
///
/// `base.revision == current_revision` always succeeds (fresh). `base.revision` greater
/// than current is rejected as unavailable/skew.
pub fn conflict_field(
    base_revision: u64,
    current_revision: u64,
    edit: &ConfigEdit,
    field_revisions: &std::collections::BTreeMap<FieldPath, u64>,
) -> Option<FieldPath> {
    if base_revision > current_revision {
        return Some("config".into());
    }
    if base_revision == current_revision {
        return None;
    }
    let writes = write_set(edit);
    // RepairActivation has an empty write set — never a config-field conflict.
    for write in &writes {
        for (written, &rev) in field_revisions {
            if rev > base_revision && fields_overlap(write, written) {
                return Some(write.clone());
            }
        }
    }
    None
}

/// Record that `writes` were committed at `revision`.
pub fn note_writes(
    field_revisions: &mut std::collections::BTreeMap<FieldPath, u64>,
    writes: &[FieldPath],
    revision: u64,
) {
    for w in writes {
        field_revisions
            .entry(w.clone())
            .and_modify(|last| *last = (*last).max(revision))
            .or_insert(revision);
    }
}

#[cfg(test)]
mod conflict_tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn unrelated_fields_do_not_conflict() {
        let mut revs = BTreeMap::new();
        revs.insert("debug".into(), 2);
        let edit = ConfigEdit::Toggle {
            field: ConfigToggle::AutoHideName,
            enabled: true,
        };
        assert_eq!(conflict_field(1, 2, &edit, &revs), None);
    }

    #[test]
    fn same_field_conflicts() {
        let mut revs = BTreeMap::new();
        revs.insert("debug".into(), 2);
        let edit = ConfigEdit::Toggle {
            field: ConfigToggle::Debug,
            enabled: false,
        };
        assert_eq!(conflict_field(1, 2, &edit, &revs).as_deref(), Some("debug"));
    }

    #[test]
    fn replace_conflicts_with_any_domain_change() {
        let mut revs = BTreeMap::new();
        revs.insert("debug".into(), 2);
        let edit = ConfigEdit::Replace {
            config: CanonicalConfig::default(),
        };
        assert!(conflict_field(1, 2, &edit, &revs).is_some());
    }

    #[test]
    fn remove_app_overlaps_app_field() {
        assert!(fields_overlap("apps/com.x", "apps/com.x/native"));
        let mut revs = BTreeMap::new();
        revs.insert("apps/com.x/native".into(), 3);
        let edit = ConfigEdit::RemoveApp {
            pkg: "com.x".into(),
        };
        assert_eq!(
            conflict_field(1, 3, &edit, &revs).as_deref(),
            Some("apps/com.x")
        );
    }

    #[test]
    fn repair_activation_never_field_conflicts() {
        let mut revs = BTreeMap::new();
        revs.insert("debug".into(), 9);
        assert_eq!(
            conflict_field(1, 9, &ConfigEdit::RepairActivation, &revs),
            None
        );
    }
}
