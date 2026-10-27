//! The APK submits one atomic field-intent batch; ordinary edits never replace
//! unrelated canonical domains. Test admission, persistence and restart together.

use vpnhide_core_daemon::{CoreState, Publisher};
use vpnhide_core_rpc::{
    BaseRef, CanonicalConfig, ConfigEdit, ConfigToggle, OperationId, PhaseOutcome, StateSnapshot,
    SubmitError,
};

fn toggle(field: ConfigToggle, enabled: bool) -> ConfigEdit {
    ConfigEdit::Toggle { field, enabled }
}

#[test]
fn a_batch_commits_once_merges_unrelated_edits_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.json");
    let journal = dir.path().join("operations.json");
    let load = |epoch| {
        CoreState::load_with_journal(
            Publisher::new(epoch, StateSnapshot::default()),
            config.clone(),
            Some(journal.clone()),
            None,
        )
        .unwrap()
    };
    let state = load(1);
    for _ in 0..4 {
        state.issue_operation_id().unwrap();
    }
    state
        .submit_edit_public(
            OperationId(1),
            BaseRef { revision: 0 },
            &ConfigEdit::Replace {
                config: CanonicalConfig::default(),
            },
        )
        .unwrap();
    let base = BaseRef { revision: 1 };
    // Another client changes a field the editor has not touched.
    state
        .submit_edit_public(
            OperationId(2),
            base,
            &toggle(ConfigToggle::AutoHideName, true),
        )
        .unwrap();
    let edit = ConfigEdit::Batch {
        edits: vec![
            toggle(ConfigToggle::Debug, true),
            toggle(ConfigToggle::RememberSuperkey, true),
        ],
    };
    state
        .submit_edit_public(OperationId(3), base, &edit)
        .unwrap();
    assert_eq!(
        state.config_revision(),
        3,
        "one revision for the entire batch"
    );
    let current = state.publisher.current().snapshot;
    let value = current.config.unwrap();
    assert!(value.debug && value.settings.remember_superkey && value.settings.auto_hide_vpn_name);
    assert!(current.operations_in_flight.is_empty());
    assert!(!current.mutation_held);
    drop(state);

    let restarted = load(2);
    restarted
        .submit_edit_public(OperationId(3), base, &edit)
        .expect("same original request queries the durable receipt");
    assert_eq!(restarted.config_revision(), 3);
    assert_eq!(
        restarted
            .get_operation_record(OperationId(3))
            .unwrap()
            .settled,
        Some(PhaseOutcome::Confirmed)
    );
    assert!(matches!(
        restarted.submit_edit_public(
            OperationId(4),
            base,
            &ConfigEdit::Batch {
                edits: vec![
                    toggle(ConfigToggle::Debug, false),
                    toggle(ConfigToggle::AutoHideServices, false),
                ],
            }
        ),
        Err(SubmitError::Conflict { .. })
    ));
    let after = restarted.publisher.current().snapshot.config.unwrap();
    assert!(
        after.debug && after.settings.auto_hide_vpn_services,
        "no partial batch"
    );
}

#[test]
fn a_batch_cannot_hide_import_repair_or_nested_batches() {
    let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
    let broad = [
        ConfigEdit::Replace {
            config: CanonicalConfig::default(),
        },
        ConfigEdit::RepairActivation,
        ConfigEdit::Batch { edits: Vec::new() },
    ];
    for edit in broad {
        assert!(matches!(
            state.submit_edit_public(
                state.issue_operation_id().unwrap(),
                BaseRef { revision: 0 },
                &ConfigEdit::Batch { edits: vec![edit] },
            ),
            Err(SubmitError::Rejected { .. })
        ));
    }
    assert_eq!(state.config_revision(), 0);
    assert!(state.publisher.current().snapshot.config.is_none());
}

#[test]
fn direct_rpc_import_cannot_bypass_canonical_schema_validation() {
    let state = CoreState::new(Publisher::new(1, StateSnapshot::default()));
    for _ in 0..4 {
        state.issue_operation_id().unwrap();
    }
    assert!(matches!(
        state.submit_edit_public(
            OperationId(1),
            BaseRef { revision: 0 },
            &ConfigEdit::Replace {
                config: CanonicalConfig {
                    version: 2,
                    ..CanonicalConfig::default()
                },
            }
        ),
        Err(SubmitError::Rejected { .. })
    ));
    assert_eq!(state.config_revision(), 0);
    assert!(state.get_operation_record(OperationId(1)).is_none());
}
