//! Real writer, canonical and journal behavior without opening an APK process.
use std::{fs, path::Path, sync::Arc};
use vpnhide_core_daemon::{CoreState, Publisher, config_file, effects::EffectPaths, journal};
use vpnhide_core_rpc::{
    AppConfig, BaseRef, CanonicalConfig, ConfigEdit, ConfigPhase, CoreCommand, PhaseOutcome,
    StateSnapshot, SubmitError,
};
use vpnhide_migration_policy::APP_PACKAGE;

fn load(root: &Path) -> Arc<CoreState> {
    let state = CoreState::load_with_journal(
        Publisher::new(8, StateSnapshot::default()),
        root.join("config.json"),
        Some(root.join("journal.json")),
        None,
    )
    .unwrap();
    let effects = root.join("legacy-root");
    fs::create_dir_all(&effects).unwrap();
    state.set_effect_paths(Some(EffectPaths { root: effects }));
    state.configure_autonomous_diagnostics(root.join("app"));
    state
}

#[test]
fn boot_imports_known_choices_and_self_without_app_or_legacy_deletion() {
    let root = tempfile::tempdir().unwrap();
    let source = root
        .path()
        .join("legacy-root/data/adb/vpnhide_kmod/targets.txt");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(&source, "com.bank.app\n").unwrap();
    let state = load(root.path());
    let op = state.prepare_boot_self_targets().unwrap().unwrap();
    let config = config_file::read(&root.path().join("config.json"))
        .unwrap()
        .unwrap();
    assert!(config.apps["com.bank.app"].native.enabled);
    let app = &config.apps[APP_PACKAGE];
    assert!(app.java && app.native.enabled && app.hidden && !app.ports);
    assert_eq!(app.java_hooks, None);
    assert_eq!(fs::read_to_string(&source).unwrap(), "com.bank.app\n");
    let entry = state.get_operation_record(op).unwrap();
    assert_eq!(entry.settled, Some(PhaseOutcome::Confirmed));
    assert_eq!(
        entry.phases.iter().map(|p| p.phase).collect::<Vec<_>>(),
        vec![ConfigPhase::Persist]
    );
    let persisted = journal::read(&root.path().join("journal.json")).unwrap();
    assert_eq!(persisted.version, vpnhide_build_contract::JOURNAL_VERSION);
    assert_eq!(
        persisted.get(op).unwrap().command,
        Some(journal::CommandIntent::PrepareSelfTargets)
    );
    state.shutdown();
    let restarted = load(root.path());
    assert_eq!(restarted.prepare_boot_self_targets().unwrap(), None);
    assert_eq!(restarted.snapshot().config, Some(config));
    restarted.shutdown();
}

#[test]
fn configured_apps_need_no_legacy_reader_and_keep_user_intent() {
    let root = tempfile::tempdir().unwrap();
    let state = load(root.path());
    let original = CanonicalConfig {
        debug: true,
        debug_switch: false,
        apps: std::collections::BTreeMap::from([(
            "com.bank.app".into(),
            AppConfig {
                java: true,
                java_hooks: Some(vec!["selected".into()]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let op = state.issue_operation_id().unwrap();
    state
        .submit_edit_public(
            op,
            BaseRef { revision: 0 },
            &ConfigEdit::Replace {
                config: original.clone(),
            },
        )
        .unwrap();
    state.set_effect_paths(None);
    state.prepare_boot_self_targets().unwrap();
    let result = state.snapshot().config.unwrap();
    assert_eq!(result.apps["com.bank.app"], original.apps["com.bank.app"]);
    assert!(result.debug && !result.debug_switch);
    assert_eq!(state.prepare_boot_self_targets().unwrap(), None);
    state.shutdown();
}

#[test]
fn malformed_legacy_cannot_become_an_empty_bootstrap() {
    let root = tempfile::tempdir().unwrap();
    let source = root
        .path()
        .join("legacy-root/data/adb/vpnhide_kmod/targets.txt");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(&source, "this is not a package\n").unwrap();
    let state = load(root.path());
    assert!(matches!(
        state.prepare_boot_self_targets(),
        Err(SubmitError::Rejected { .. })
    ));
    assert!(!root.path().join("config.json").exists());
    assert_eq!(
        fs::read_to_string(source).unwrap(),
        "this is not a package\n"
    );
    state.shutdown();
}

#[test]
fn schema_eleven_persist_hold_survives_upgrade_and_blocks_bootstrap() {
    let root = tempfile::tempdir().unwrap();
    let old = journal::Journal {
        version: 11,
        expired_holds: vec![ConfigPhase::Persist],
        issued_through: 50,
        ..Default::default()
    };
    journal::write_atomic(&root.path().join("journal.json"), &old).unwrap();
    let state = load(root.path());
    assert!(state.prepare_boot_self_targets().is_err());
    assert!(!root.path().join("config.json").exists());
    assert!(state.has_unknown_hold());
    let saved = journal::read(&root.path().join("journal.json")).unwrap();
    assert_eq!(saved.expired_holds, old.expired_holds);
    assert!(saved.issued_through >= old.issued_through);
    state.shutdown();
}

#[test]
fn same_operation_is_historical_after_reset_and_never_replays_bootstrap() {
    let root = tempfile::tempdir().unwrap();
    let state = load(root.path());
    let op = state.issue_operation_id().unwrap();
    state
        .submit_command(
            op,
            CoreCommand::PrepareSelfTargets {
                base: BaseRef { revision: 0 },
            },
        )
        .unwrap();
    assert!(state.has_config());
    let reset = state.issue_operation_id().unwrap();
    state.submit_command(reset, CoreCommand::FullReset).unwrap();
    assert!(!state.has_config());
    state
        .submit_command(
            op,
            CoreCommand::PrepareSelfTargets {
                base: BaseRef { revision: 0 },
            },
        )
        .unwrap();
    assert!(!state.has_config());
    assert!(matches!(
        state.submit_command(op, CoreCommand::CleanupLegacy),
        Err(SubmitError::IdReused)
    ));
    state.shutdown();
}
