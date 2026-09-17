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
    }
    next
}

fn apply_toggle(config: &mut CanonicalConfig, field: ConfigToggle, enabled: bool) {
    match field {
        ConfigToggle::Filesystem => {
            if enabled {
                config.settings.optional_features.insert(FILESYSTEM_FEATURE.to_owned());
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
        AppConfig { native: NativeRole { enabled: native, ..Default::default() }, ..Default::default() }
    }

    #[test]
    fn toggles_set_their_field() {
        let mut c = empty();
        c = apply_canonical_edit(&c, &ConfigEdit::Toggle { field: ConfigToggle::Debug, enabled: true });
        assert!(c.debug);
        c = apply_canonical_edit(&c, &ConfigEdit::Toggle { field: ConfigToggle::AutoHideName, enabled: true });
        assert!(c.settings.auto_hide_vpn_name);
    }

    #[test]
    fn filesystem_toggle_adds_and_removes_the_feature() {
        let on = apply_canonical_edit(&empty(), &ConfigEdit::Toggle { field: ConfigToggle::Filesystem, enabled: true });
        assert!(on.settings.optional_features.contains(FILESYSTEM_FEATURE));
        let off = apply_canonical_edit(&on, &ConfigEdit::Toggle { field: ConfigToggle::Filesystem, enabled: false });
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
        assert_eq!(c.settings.auto_hide_excluded_packages, BTreeSet::from(["com.bank".to_owned()]));
    }

    #[test]
    fn app_edit_merges_only_the_named_field() {
        // Start with an app that hides; edit only its Native field.
        let mut c = empty();
        c.apps.insert("com.x".into(), AppConfig { hidden: true, ..Default::default() });
        let edited = apply_canonical_edit(
            &c,
            &ConfigEdit::App { pkg: "com.x".into(), field: ConfigAppField::Native, value: app_with(true) },
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
            &ConfigEdit::App { pkg: "com.x".into(), field: ConfigAppField::Native, value: app_with(false) },
        );
        assert!(!edited.apps.contains_key("com.x"));
    }

    #[test]
    fn remove_app_drops_it() {
        let mut c = empty();
        c.apps.insert("com.x".into(), app_with(true));
        let edited = apply_canonical_edit(&c, &ConfigEdit::RemoveApp { pkg: "com.x".into() });
        assert!(edited.apps.is_empty());
    }

    #[test]
    fn replace_swaps_the_whole_config() {
        let replacement = CanonicalConfig { debug: true, settings: Settings { auto_hide_vpn_name: true, ..Default::default() }, ..Default::default() };
        let edited = apply_canonical_edit(&empty(), &ConfigEdit::Replace { config: replacement.clone() });
        assert_eq!(edited, replacement);
    }

    #[test]
    fn applying_against_fresh_merges_unrelated_fields() {
        // Two edits to different fields compose without clobbering each other.
        let a = apply_canonical_edit(&empty(), &ConfigEdit::Toggle { field: ConfigToggle::Debug, enabled: true });
        let b = apply_canonical_edit(&a, &ConfigEdit::Toggle { field: ConfigToggle::RememberSuperkey, enabled: true });
        assert!(b.debug && b.settings.remember_superkey);
    }
}
