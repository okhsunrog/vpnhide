use std::collections::BTreeMap;
use vpnhide_core_rpc::{
    AppConfig, CanonicalConfig, ConfigAppField, ConfigEdit, ConfigSetField, ConfigToggle,
    NativeRole,
    config_edit::{FILESYSTEM_FEATURE, apply_canonical_edit, diff_fields, write_set},
};

#[test]
fn diff_is_a_field_delta_and_does_not_erase_unrelated_fresh_roles() {
    let base = CanonicalConfig {
        apps: BTreeMap::from([(
            "com.bank".into(),
            AppConfig {
                java: true,
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut desired = base.clone();
    desired.apps.clear();
    desired
        .settings
        .optional_features
        .insert(FILESYSTEM_FEATURE.into());
    let edits = diff_fields(&base, &desired);
    assert_eq!(edits.len(), 2);
    assert!(
        matches!(&edits[0], ConfigEdit::SettingSet { field: ConfigSetField::OptionalFeatures, values } if values.contains(FILESYSTEM_FEATURE))
    );
    assert!(
        matches!(&edits[1], ConfigEdit::App { pkg, field: ConfigAppField::Java, value } if pkg == "com.bank" && !value.java)
    );
    let mut fresh = base;
    fresh.apps.get_mut("com.bank").unwrap().native = NativeRole {
        enabled: true,
        ..Default::default()
    };
    fresh.settings.auto_hide_vpn_name = true;
    let updated = edits
        .iter()
        .fold(fresh, |config, edit| apply_canonical_edit(&config, edit));
    assert!(!updated.apps["com.bank"].java);
    assert!(updated.apps["com.bank"].native.enabled);
    assert!(updated.settings.auto_hide_vpn_name);
    assert!(
        updated
            .settings
            .optional_features
            .contains(FILESYSTEM_FEATURE)
    );
}

#[test]
fn empty_delta_does_not_remove_an_explicit_same_value_write_intent() {
    let config = CanonicalConfig::default();
    assert!(diff_fields(&config, &config).is_empty());
    let intent = ConfigEdit::Toggle {
        field: ConfigToggle::DebugSwitch,
        enabled: false,
    };
    assert_eq!(write_set(&intent), vec!["debugSwitch"]);
    assert_eq!(apply_canonical_edit(&config, &intent), config);
}
