use super::*;

fn selected(names: &[&str]) -> BTreeMap<String, NativeSelection> {
    names
        .iter()
        .map(|name| ((*name).into(), NativeSelection::Enabled(true)))
        .collect()
}

#[test]
fn shared_uids_profiles_platform_ids_and_real_main_self_are_counted_once() {
    let packages = BTreeMap::from([
        (APP_PACKAGE.into(), vec![10123, 110123]),
        ("one".into(), vec![12000, 112000, 1000, 101000]),
        ("shared".into(), vec![12000]),
    ]);
    let selections = selected(&[APP_PACKAGE, "one", "shared", "uninstalled"]);
    let set = native_target_set(&selections, &packages, NativeHookFamily::Kpm, false);
    assert_eq!(
        set.keys().copied().collect::<Vec<_>>(),
        [10123, 12000, 112000]
    );
    assert_eq!(
        set[&12000].1,
        BTreeSet::from(["one".into(), "shared".into()])
    );
    let absent_self =
        native_target_set(&selections, &BTreeMap::new(), NativeHookFamily::Kpm, false);
    assert!(absent_self.is_empty());
}

#[test]
fn self_sharing_uid_does_not_spend_a_synthetic_reserved_slot() {
    let packages = BTreeMap::from([
        (APP_PACKAGE.into(), vec![10123]),
        ("shared".into(), vec![10123]),
    ]);
    assert_eq!(
        usage(
            false,
            &selected(&[APP_PACKAGE, "shared"]),
            &packages,
            NativeHookFamily::Kpm,
            false
        )
        .target_uids,
        1
    );
}

#[test]
fn empty_family_masks_and_disabled_filesystem_only_masks_spend_no_slots() {
    let selections = BTreeMap::from([
        (
            "filesystem".into(),
            NativeSelection::Detailed(NativeSelectionDetail {
                enabled: true,
                kernel: Some(vec![FILESYSTEM_FEATURE.into()]),
                zygisk: Some(vec![]),
            }),
        ),
        (
            "empty".into(),
            NativeSelection::Detailed(NativeSelectionDetail {
                enabled: true,
                kernel: Some(vec![]),
                zygisk: None,
            }),
        ),
    ]);
    let packages = BTreeMap::from([
        ("filesystem".into(), vec![12000]),
        ("empty".into(), vec![12001]),
    ]);
    assert_eq!(
        usage(
            false,
            &selections,
            &packages,
            NativeHookFamily::Builtin,
            false
        )
        .target_uids,
        0
    );
    assert_eq!(
        usage(
            false,
            &selections,
            &packages,
            NativeHookFamily::Builtin,
            true
        )
        .target_uids,
        1
    );
    // The ko has a separate load-time gate and preserves the configured bit.
    assert_eq!(
        usage(false, &selections, &packages, NativeHookFamily::Kmod, false).target_uids,
        1
    );
    assert_eq!(
        usage(
            false,
            &selections,
            &packages,
            NativeHookFamily::Zygisk,
            false
        )
        .target_uids,
        1
    );
}

#[test]
fn preview_retains_overflow_usage_while_delivery_refuses_the_whole_projection() {
    let selections = selected(&["many"]);
    let capacity = NativeHookFamily::Kpm.limits().target_uids;
    for count in [capacity, capacity + 1] {
        let packages = BTreeMap::from([("many".into(), (10000..).take(count).collect())]);
        let preview = usage(false, &selections, &packages, NativeHookFamily::Kpm, false);
        assert_eq!(preview.target_uids, count);
        let wire = project_native(false, &selections, &packages, NativeHookFamily::Kpm, false);
        assert_eq!(wire.is_ok(), count <= capacity);
        if let Ok(wire) = wire {
            assert_eq!(preview.wire_bytes, wire.len());
        }
    }
}

#[test]
fn kpm_wire_can_overflow_before_the_uid_limit_and_other_families_keep_their_own_limits() {
    let capacity = NativeHookFamily::Kpm.limits().target_uids;
    let packages = BTreeMap::from([(
        "wide".into(),
        (0..capacity as u32).map(|i| 4_000_010_000 + i).collect(),
    )]);
    let selections = selected(&["wide"]);
    let kpm = usage(false, &selections, &packages, NativeHookFamily::Kpm, false);
    assert_eq!(kpm.target_uids, capacity);
    assert!(kpm.wire_bytes > kpm.limits.wire_bytes);
    assert!(!kpm.fits());
    assert!(usage(false, &selections, &packages, NativeHookFamily::Kmod, false).fits());
}

#[test]
fn a_real_policy_reduction_can_increase_grouped_bytes_and_must_remain_allowed() {
    let both = NativeSelection::Hooks(vec![
        Hook::DevIoctl.name().into(),
        Hook::SockIoctl.name().into(),
    ]);
    let before = BTreeMap::from([
        ("a".into(), both.clone()),
        (
            "b".into(),
            NativeSelection::Hooks(vec![Hook::SockIoctl.name().into()]),
        ),
        ("c".into(), both),
    ]);
    let packages = BTreeMap::from([
        ("a".into(), (2_010_000..2_010_051).collect()),
        ("b".into(), (2_010_000..2_010_050).collect()),
        ("c".into(), (2_010_051..2_010_150).collect()),
    ]);
    let mut after = before.clone();
    after.remove("a");
    let old = usage(false, &before, &packages, NativeHookFamily::Kpm, false);
    let new = usage(false, &after, &packages, NativeHookFamily::Kpm, false);
    assert!(new.target_uids < old.target_uids);
    assert!(new.wire_bytes > old.wire_bytes);
    assert!(!old.fits() && !new.fits());
    assert!(is_reduction(
        &before,
        false,
        &after,
        false,
        NativeHookFamily::Kpm
    ));
    assert!(!is_reduction(
        &after,
        false,
        &before,
        false,
        NativeHookFamily::Kpm
    ));
}

#[test]
fn required_installation_hooks_come_from_desired_roles_without_inventory_or_observed_mask() {
    let mut roles = BTreeMap::new();
    roles.insert("unresolved.app".into(), NativeSelection::Enabled(true));
    for family in [
        NativeHookFamily::Builtin,
        NativeHookFamily::Kmod,
        NativeHookFamily::Kpm,
        NativeHookFamily::Zygisk,
    ] {
        let off = required_hooks(&roles, family, false);
        let on = required_hooks(&roles, family, true);
        assert_ne!(off, 0);
        assert_eq!(off & Hook::FilesystemIfacePaths.bit(), 0);
        assert_ne!(on & Hook::FilesystemIfacePaths.bit(), 0);
    }
    roles.insert("unresolved.app".into(), NativeSelection::Enabled(false));
    assert_eq!(required_hooks(&roles, NativeHookFamily::Builtin, true), 0);
}
