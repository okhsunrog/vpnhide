use super::*;
use NativeBackend::{Builtin, Kmod, Kpm, Zygisk};
use std::cell::Cell;

fn facts(installed: &[NativeBackend], kernel: KernelFact, kpm: KpmFact) -> ScanFacts {
    ScanFacts {
        installed: installed.to_vec(),
        staged: Vec::new(),
        kernel,
        kpm,
        kpm_may_be_live: false,
        kpm_filesystem_hiding: None,
    }
}

/// One way each backend can be a candidate: an installed module, or a live identity.
#[derive(Clone, Copy, Debug)]
enum Source {
    KmodModule,
    KpmModule,
    ZygiskModule,
    BuiltinKernel,
    LiveKpm,
}

fn apply(sources: &[Source]) -> ScanFacts {
    let mut f = facts(&[], KernelFact::Absent, KpmFact::Absent);
    for s in sources {
        match s {
            Source::KmodModule => f.installed.push(Kmod),
            Source::KpmModule => f.installed.push(Kpm),
            Source::ZygiskModule => f.installed.push(Zygisk),
            Source::BuiltinKernel => f.kernel = KernelFact::Builtin,
            Source::LiveKpm => f.kpm = KpmFact::Live,
        }
    }
    f
}

fn backend_of(s: Source) -> NativeBackend {
    match s {
        Source::KmodModule => Kmod,
        Source::KpmModule | Source::LiveKpm => Kpm,
        Source::ZygiskModule => Zygisk,
        Source::BuiltinKernel => Builtin,
    }
}

#[test]
fn a_single_candidate_is_loaded_or_adopted() {
    assert_eq!(decide(&apply(&[Source::KmodModule])), Plan::Load(Kmod));
    assert_eq!(decide(&apply(&[Source::KpmModule])), Plan::Load(Kpm));
    assert_eq!(decide(&apply(&[Source::ZygiskModule])), Plan::Adopt(Zygisk));
    assert_eq!(
        decide(&apply(&[Source::BuiltinKernel])),
        Plan::Adopt(Builtin)
    );
    // A live KPM core did not load (APatch autoload / standalone .kpm), alone.
    assert_eq!(decide(&apply(&[Source::LiveKpm])), Plan::Adopt(Kpm));
    // A live kmod with no module (loaded by hand) is adopted, never re-loaded.
    assert_eq!(
        decide(&facts(&[], KernelFact::Kmod, KpmFact::Absent)),
        Plan::Adopt(Kmod)
    );
}

#[test]
fn every_pair_of_distinct_backends_is_a_conflict_that_loads_nothing() {
    let all = [
        Source::KmodModule,
        Source::KpmModule,
        Source::ZygiskModule,
        Source::BuiltinKernel,
        Source::LiveKpm,
    ];
    for (i, &a) in all.iter().enumerate() {
        for &b in &all[i + 1..] {
            if backend_of(a) == backend_of(b) {
                continue; // KPM module + live KPM: one backend (see below).
            }
            let f = apply(&[a, b]);
            match decide(&f) {
                Plan::Stop(NativeBootOutcome::Conflict {
                    installed,
                    live,
                    staged,
                }) => {
                    let mut named: Vec<_> = installed.iter().chain(live.iter()).copied().collect();
                    named.dedup();
                    assert_eq!(
                        ordered(named),
                        ordered([backend_of(a), backend_of(b)]),
                        "{a:?}+{b:?}"
                    );
                    assert!(staged.is_empty());
                }
                other => panic!("{a:?}+{b:?} must conflict, got {other:?}"),
            }
        }
    }
}

#[test]
fn builtin_kernel_plus_a_kmod_module_is_a_conflict() {
    assert_eq!(
        decide(&apply(&[Source::BuiltinKernel, Source::KmodModule])),
        Plan::Stop(NativeBootOutcome::Conflict {
            installed: vec![Kmod],
            live: vec![Builtin],
            staged: vec![],
        })
    );
}

#[test]
fn three_backends_conflict_with_all_named() {
    let f = apply(&[Source::KmodModule, Source::KpmModule, Source::ZygiskModule]);
    assert_eq!(
        decide(&f),
        Plan::Stop(NativeBootOutcome::Conflict {
            installed: vec![Kmod, Kpm, Zygisk],
            live: vec![],
            staged: vec![],
        })
    );
}

#[test]
fn a_module_and_its_own_live_runtime_is_one_backend() {
    // Core restart within a boot whose decision file is gone: same family, adopted.
    assert_eq!(
        decide(&facts(&[Kmod], KernelFact::Kmod, KpmFact::Absent)),
        Plan::Adopt(Kmod)
    );
    assert_eq!(
        decide(&facts(&[Kpm], KernelFact::Absent, KpmFact::Live)),
        Plan::Adopt(Kpm)
    );
}

#[test]
fn staged_modules_are_reported_but_never_candidates() {
    let mut f = facts(&[Kmod], KernelFact::Absent, KpmFact::Absent);
    f.staged = vec![Kpm, Zygisk];
    assert_eq!(decide(&f), Plan::Load(Kmod));
    let mut f = facts(&[], KernelFact::Absent, KpmFact::Absent);
    f.staged = vec![Kpm];
    assert_eq!(decide(&f), Plan::Stop(NativeBootOutcome::NoBackend));
    // A conflict lists what is staged too.
    let mut f = facts(&[Kmod, Zygisk], KernelFact::Absent, KpmFact::Absent);
    f.staged = vec![Kpm];
    assert_eq!(
        decide(&f),
        Plan::Stop(NativeBootOutcome::Conflict {
            installed: vec![Kmod, Zygisk],
            live: vec![],
            staged: vec![Kpm],
        })
    );
}

#[test]
fn nothing_installed_or_live_is_no_backend() {
    assert_eq!(
        decide(&facts(&[], KernelFact::Absent, KpmFact::NoRuntime)),
        Plan::Stop(NativeBootOutcome::NoBackend)
    );
}

#[test]
fn foreign_or_unreadable_control_node_blocks_everything() {
    assert_eq!(
        decide(&facts(&[Kmod], KernelFact::Foreign(0x99), KpmFact::Absent)),
        Plan::Stop(NativeBootOutcome::ForeignOwner { id: 0x99 })
    );
    assert!(matches!(
        decide(&facts(
            &[Kmod],
            KernelFact::Unreadable("EACCES".into()),
            KpmFact::Absent
        )),
        Plan::Stop(NativeBootOutcome::Unknown { .. })
    ));
}

#[test]
fn kpm_auth_failure_blocks_only_a_kpm_candidate_or_a_possibly_live_kpm() {
    let auth = || KpmFact::AuthFailed("no superkey".into());
    // Without a KPM candidate and without evidence: kmod / builtin / Zygisk proceed.
    assert_eq!(
        decide(&facts(&[Kmod], KernelFact::Absent, auth())),
        Plan::Load(Kmod)
    );
    assert_eq!(
        decide(&facts(&[], KernelFact::Builtin, auth())),
        Plan::Adopt(Builtin)
    );
    assert_eq!(
        decide(&facts(&[Zygisk], KernelFact::Absent, auth())),
        Plan::Adopt(Zygisk)
    );
    assert_eq!(
        decide(&facts(&[], KernelFact::Absent, auth())),
        Plan::Stop(NativeBootOutcome::AwaitingAuthentication {
            detail: "no superkey".into()
        }),
        "nothing else to select: only an unseen KPM could exist, so say why"
    );
    // The KPM is the candidate: awaiting the superkey.
    assert_eq!(
        decide(&facts(&[Kpm], KernelFact::Absent, auth())),
        Plan::Stop(NativeBootOutcome::AwaitingAuthentication {
            detail: "no superkey".into()
        })
    );
    // KPM plus another module is a conflict regardless of authentication.
    assert!(matches!(
        decide(&facts(&[Kmod, Kpm], KernelFact::Absent, auth())),
        Plan::Stop(NativeBootOutcome::Conflict { .. })
    ));
    // A VPN Hide KPM may be live (APatch-installed): nothing else may load.
    let mut f = facts(&[Kmod], KernelFact::Absent, auth());
    f.kpm_may_be_live = true;
    assert_eq!(
        decide(&f),
        Plan::Stop(NativeBootOutcome::KpmUnverifiable {
            detail: "no superkey".into()
        })
    );
    // A broken (unenumerable) runtime with a KPM candidate is unverifiable.
    assert_eq!(
        decide(&facts(
            &[Kpm],
            KernelFact::Absent,
            KpmFact::Broken("list failed".into())
        )),
        Plan::Stop(NativeBootOutcome::KpmUnverifiable {
            detail: "list failed".into()
        })
    );
}

// ---- effects ----------------------------------------------------------------

struct Fake {
    ctl: PathBuf,
    insmod_result: Option<KmodLoadFailure>,
    insmods: Cell<u32>,
}

impl Effects for Fake {
    fn insmod(&self, _: &Path, _: &Path, fs: bool) -> Result<(), KmodLoadFailure> {
        self.insmods.set(self.insmods.get() + 1);
        assert!(fs);
        match &self.insmod_result {
            Some(f) => Err(f.clone()),
            None => {
                write_ctl(&self.ctl, "0");
                Ok(())
            }
        }
    }
    fn kpm_load(&self, _: &CoreKpm, _: &Path, _: &Path, _: bool) -> Result<(), String> {
        panic!("no KPM in host tests")
    }
    fn kpm_kernel(&self) -> Result<(String, bool), String> {
        Ok(("6.1.0".into(), true))
    }
    fn kprobes(&self) -> (Option<bool>, Option<bool>) {
        (Some(true), Some(false))
    }
    fn dmesg_tail(&self) -> Vec<String> {
        vec!["vpnhide: module verification failed".into()]
    }
    fn uname(&self) -> String {
        "6.1.0-android14".into()
    }
}

fn write_ctl(path: &Path, backend_hex: &str) {
    fs::write(
        path,
        format!(
            "vpnhide 1 status\nbackend 0x{backend_hex}\nkver 0x60100\nhooks 0x3ff\nerror 0x0\n"
        ),
    )
    .unwrap();
}

fn env(root: &Path) -> NativeEnv {
    fs::create_dir_all(root.join("modules")).unwrap();
    fs::write(root.join("boot_id"), "boot-1\n").unwrap();
    NativeEnv {
        ctl: root.join("ctl"),
        modules: Some(root.join("modules")),
        kpm_lock: None,
        zygisk_module: None,
        boot_id_path: root.join("boot_id"),
        state_file: root.join("core/native_boot.json"),
        legacy_root: Some(root.join("legacy")),
        apatch_kpms: root.join("ap/kpms"),
    }
}

fn install(root: &Path, id: &str, artifact: Option<&str>) -> PathBuf {
    let dir = root.join("modules").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("module.prop"),
        format!("id={id}\nversion=v2.0.0\ngkiVariant=android14-6.1\n"),
    )
    .unwrap();
    if let Some(name) = artifact {
        fs::write(dir.join(name), "artifact").unwrap();
    }
    dir
}

fn fake(env: &NativeEnv, insmod_result: Option<KmodLoadFailure>) -> Fake {
    Fake {
        ctl: env.ctl.clone(),
        insmod_result,
        insmods: Cell::new(0),
    }
}

#[test]
fn gather_counts_installed_disabled_removed_and_staged_modules() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    install(dir.path(), "vpnhide_kmod", Some(KMOD_ARTIFACT));
    let kpm = install(dir.path(), "vpnhide_kpm", Some(KPM_ARTIFACT));
    fs::write(kpm.join("disable"), "").unwrap();
    let zygisk = install(dir.path(), "vpnhide_zygisk", None);
    fs::write(zygisk.join("remove"), "").unwrap();
    let f = gather(&env, KpmFact::NotManaged);
    assert_eq!(f.installed, vec![Kmod], "disabled/remove never count");
    assert!(
        f.staged.is_empty(),
        "an existing (disabled) module dir is not staged-only"
    );

    // A new install sitting only in modules_update is staged, not installed.
    fs::remove_dir_all(&kpm).unwrap();
    let staged = dir.path().join("modules_update/vpnhide_kpm");
    fs::create_dir_all(&staged).unwrap();
    fs::write(staged.join("module.prop"), "id=vpnhide_kpm\n").unwrap();
    let f = gather(&env, KpmFact::NotManaged);
    assert_eq!(f.installed, vec![Kmod]);
    assert_eq!(f.staged, vec![Kpm]);
    assert_eq!(f.kernel, KernelFact::Absent);

    write_ctl(&env.ctl, "4");
    assert_eq!(
        gather(&env, KpmFact::NotManaged).kernel,
        KernelFact::Builtin
    );
    write_ctl(&env.ctl, "99");
    assert_eq!(
        gather(&env, KpmFact::NotManaged).kernel,
        KernelFact::Foreign(0x99)
    );
}

#[test]
fn apatch_installed_vpnhide_kpm_is_possible_liveness_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let kpms = dir.path().join("kpms/abcd");
    assert!(!apatch_installed_vpnhide_kpm(&dir.path().join("kpms")));
    fs::create_dir_all(&kpms).unwrap();
    fs::write(kpms.join("x.kpm"), b"\x7fELF...name=other\0").unwrap();
    assert!(!apatch_installed_vpnhide_kpm(&dir.path().join("kpms")));
    fs::write(kpms.join("y.kpm"), b"\x7fELF...name=vpnhide\0version=2").unwrap();
    assert!(apatch_installed_vpnhide_kpm(&dir.path().join("kpms")));
}

#[test]
fn loading_the_single_kmod_records_load_args_and_kernel_facts() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    install(dir.path(), "vpnhide_kmod", Some(KMOD_ARTIFACT));
    let fx = fake(&env, None);
    let f = gather(&env, KpmFact::NoRuntime);
    let boot = NativeBoot::decide_and_apply(env, 7, "boot-1", &f, None, true, &fx);
    assert_eq!(fx.insmods.get(), 1);
    assert_eq!(
        boot.status.outcome,
        NativeBootOutcome::Selected {
            backend: Kmod,
            mode: SelectionMode::Loaded,
            filesystem_hiding: Some(true),
        }
    );
    assert_eq!(boot.selected(), Some(Kmod));
    let kmod = boot.status.kmod.unwrap();
    assert_eq!((kmod.kprobes, kmod.kretprobes), (Some(true), Some(false)));
    assert_eq!(kmod.gki_variant, "android14-6.1");
    assert_eq!(boot.status.decided_epoch, 7);
    assert_eq!(boot.status.boot_id, "boot-1");
}

#[test]
fn a_failed_kmod_load_carries_exit_stderr_dmesg_and_signature_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    install(dir.path(), "vpnhide_kmod", Some(KMOD_ARTIFACT));
    let fx = fake(
        &env,
        Some(KmodLoadFailure {
            exit: 1,
            stderr: "insmod: Key was rejected by service".into(),
        }),
    );
    let f = gather(&env, KpmFact::NoRuntime);
    let boot = NativeBoot::decide_and_apply(env, 1, "boot-1", &f, None, true, &fx);
    match boot.status.outcome.clone() {
        NativeBootOutcome::LoadFailed {
            backend,
            exit,
            stderr,
            dmesg_tail,
            kprobes,
            kretprobes,
            signature_rejected,
        } => {
            assert_eq!(backend, Kmod);
            assert_eq!(exit, Some(1));
            assert!(stderr.contains("Key was rejected"));
            assert_eq!(dmesg_tail.len(), 1);
            assert_eq!((kprobes, kretprobes), (Some(true), Some(false)));
            assert!(signature_rejected);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        boot.selected(),
        None,
        "a failed load selects nothing to configure"
    );
}

#[test]
fn a_missing_artifact_is_reported_without_a_load() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    install(dir.path(), "vpnhide_kmod", None);
    let fx = fake(&env, None);
    let f = gather(&env, KpmFact::NoRuntime);
    let boot = NativeBoot::decide_and_apply(env, 1, "boot-1", &f, None, false, &fx);
    assert_eq!(fx.insmods.get(), 0);
    assert!(matches!(
        boot.status.outcome,
        NativeBootOutcome::MissingArtifact { backend: Kmod, .. }
    ));
}

#[test]
fn a_kpm_without_runtime_or_with_an_unsupported_kernel_reports_why() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = env(dir.path());
    install(dir.path(), "vpnhide_kpm", Some(KPM_ARTIFACT));
    env.kpm_lock = Some(dir.path().join("ctl.lock"));
    let fx = fake(&env, None);
    let f = gather(&env, KpmFact::NoRuntime);
    let boot = NativeBoot::decide_and_apply(env, 1, "boot-1", &f, None, false, &fx);
    match boot.status.outcome {
        NativeBootOutcome::LoadFailed {
            backend, stderr, ..
        } => {
            assert_eq!(backend, Kpm);
            assert!(stderr.contains("no KernelPatch runtime"), "{stderr}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_conflict_loads_nothing_and_retires_zygisk_targets() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    install(dir.path(), "vpnhide_kmod", Some(KMOD_ARTIFACT));
    let zygisk = dir.path().join("modules/vpnhide_zygisk");
    crate::zygisk::test_fixture(&zygisk);
    let fx = fake(&env, None);
    let f = gather(&env, KpmFact::NoRuntime);
    assert_eq!(f.installed, vec![Kmod, Zygisk]);
    let boot = NativeBoot::decide_and_apply(env, 1, "boot-1", &f, None, false, &fx);
    assert_eq!(fx.insmods.get(), 0, "a conflict never loads");
    assert!(matches!(
        boot.status.outcome,
        NativeBootOutcome::Conflict { .. }
    ));
    assert_eq!(boot.selected(), None, "no backend receives config");
    assert_eq!(boot.status.zygisk_targets, ZygiskTargets::Retired);
    let targets = fs::read_to_string(zygisk.join("targets.txt")).unwrap();
    assert!(targets.starts_with("vpnhide 2 config"), "{targets}");
    assert!(!targets.contains("previous policy"));
}

#[test]
fn zygisk_targets_are_retired_when_another_backend_is_selected() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    let zygisk = dir.path().join("zygisk-disabled");
    crate::zygisk::test_fixture(&zygisk);
    // Only the built-in kernel is a candidate; a disabled Zygisk is not.
    fs::write(zygisk.join("disable"), "").unwrap();
    let mut env = env;
    env.zygisk_module = Some(zygisk.clone());
    write_ctl(&env.ctl, "4");
    let fx = fake(&env, None);
    let f = gather(&env, KpmFact::NoRuntime);
    let boot = NativeBoot::decide_and_apply(env, 1, "boot-1", &f, None, false, &fx);
    assert_eq!(boot.selected(), Some(Builtin));
    assert_eq!(boot.status.zygisk_targets, ZygiskTargets::NotInstalled);

    // Zygisk selected alone: pending until config is delivered, then published.
    let dir = tempfile::tempdir().unwrap();
    let env = env_with_zygisk(dir.path());
    let fx = fake(&env, None);
    let f = gather(&env, KpmFact::NoRuntime);
    let mut boot = NativeBoot::decide_and_apply(env, 1, "boot-1", &f, None, false, &fx);
    assert_eq!(boot.selected(), Some(Zygisk));
    assert_eq!(boot.status.zygisk_targets, ZygiskTargets::Pending);
    boot.record_delivery(Ok(()));
    assert_eq!(boot.status.zygisk_targets, ZygiskTargets::Published);
}

fn env_with_zygisk(root: &Path) -> NativeEnv {
    let env = env(root);
    crate::zygisk::test_fixture(&root.join("modules/vpnhide_zygisk"));
    env
}

#[test]
fn delivery_failures_are_published_and_cleared_by_the_next_success() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    write_ctl(&env.ctl, "4");
    let fx = fake(&env, None);
    let f = gather(&env, KpmFact::NoRuntime);
    let mut boot = NativeBoot::decide_and_apply(env, 1, "boot-1", &f, None, false, &fx);
    boot.record_delivery(Err("short write".into()));
    assert_eq!(
        boot.status.outcome,
        NativeBootOutcome::DeliveryFailed {
            backend: Builtin,
            detail: "short write".into()
        }
    );
    assert_eq!(boot.selected(), Some(Builtin), "the boot selection is kept");
    boot.record_delivery(Ok(()));
    assert!(matches!(
        boot.status.outcome,
        NativeBootOutcome::Selected {
            backend: Builtin,
            ..
        }
    ));
}

#[test]
fn a_core_restart_in_the_same_boot_re_adopts_the_persisted_decision() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    install(dir.path(), "vpnhide_kmod", Some(KMOD_ARTIFACT));
    let fx = fake(&env, None);
    let f = gather(&env, KpmFact::NoRuntime);
    let boot = NativeBoot::decide_and_apply(env.clone(), 3, "boot-1", &f, None, true, &fx);
    boot.persist();
    assert_eq!(fx.insmods.get(), 1);

    // The restarted core sees the kmod it loaded live; it re-adopts the decision
    // (including the load-time filesystem_hiding) without scanning or loading.
    let again = NativeBoot::start(env.clone(), 9, false);
    assert_eq!(again.status.outcome, boot.status.outcome);
    assert_eq!(again.status.decided_epoch, 3);
    assert_eq!(again.selected(), Some(Kmod));

    // A new boot id rescans: now the live kmod plus its module is one backend, adopted.
    fs::write(&env.boot_id_path, "boot-2\n").unwrap();
    let next = NativeBoot::start(env, 10, false);
    assert_eq!(next.status.boot_id, "boot-2");
    assert_eq!(
        next.status.outcome,
        NativeBootOutcome::Selected {
            backend: Kmod,
            mode: SelectionMode::Adopted,
            filesystem_hiding: None,
        }
    );
}

#[test]
fn a_restart_keeps_a_conflict_for_the_whole_boot() {
    let dir = tempfile::tempdir().unwrap();
    let env = env_with_zygisk(dir.path());
    write_ctl(&env.ctl, "4");
    let first = NativeBoot::start(env.clone(), 1, false);
    assert!(matches!(
        first.status.outcome,
        NativeBootOutcome::Conflict { .. }
    ));
    // Even if the user removes Zygisk mid-boot, the decision stands until reboot.
    fs::write(dir.path().join("modules/vpnhide_zygisk/remove"), "").unwrap();
    let again = NativeBoot::start(env, 2, false);
    assert_eq!(again.status.outcome, first.status.outcome);
    assert_eq!(again.selected(), None);
}

#[test]
fn a_pending_authentication_is_rescanned_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    write_ctl(&env.ctl, "4");
    let pending = NativeBoot {
        env: env.clone(),
        status: NativeBootStatus {
            boot_id: "boot-1".into(),
            decided_epoch: 1,
            decided_at_ms: 0,
            updated_at_ms: 0,
            outcome: NativeBootOutcome::AwaitingAuthentication { detail: "x".into() },
            installed: vec![Kpm],
            live: vec![],
            staged: vec![],
            kmod: None,
            zygisk_targets: ZygiskTargets::NotInstalled,
            ports: None,
            obsolete_modules: vec![],
        },
        selection: None,
    };
    assert!(pending.pending_authentication());
    pending.persist();
    let again = NativeBoot::start(env, 2, false);
    assert_eq!(
        again.selected(),
        Some(Builtin),
        "re-decided from current facts"
    );
}

#[test]
fn the_boot_scan_cleans_legacy_activator_state() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    let legacy = dir.path().join("legacy/data/adb");
    for f in [
        "vpnhide_kmod/load_status",
        "vpnhide_kmod/load_dmesg",
        "vpnhide_builtin/load_status",
        "vpnhide_kpm/load_status",
        "vpnhide_kpm/ctl.lock",
        "vpnhide_ports/load_status",
        "vpnhide_ports/load_log",
    ] {
        fs::create_dir_all(legacy.join(f).parent().unwrap()).unwrap();
        fs::write(legacy.join(f), "x").unwrap();
    }
    write_ctl(&env.ctl, "4");
    NativeBoot::start(env.clone(), 1, false);
    for d in [
        "vpnhide_kmod",
        "vpnhide_builtin",
        "vpnhide_kpm",
        "vpnhide_ports",
    ] {
        assert!(
            !legacy.join(d).exists(),
            "{d} removed with its legacy state"
        );
    }

    // An installed KPM keeps its control lock.
    fs::create_dir_all(legacy.join("vpnhide_kpm")).unwrap();
    fs::write(legacy.join("vpnhide_kpm/ctl.lock"), "").unwrap();
    // An unimported 1.x Ports observer list keeps its dir; the status files still go.
    fs::create_dir_all(legacy.join("vpnhide_ports")).unwrap();
    fs::write(legacy.join("vpnhide_ports/observers.txt"), "com.example\n").unwrap();
    fs::write(legacy.join("vpnhide_ports/load_log"), "x").unwrap();
    install(dir.path(), "vpnhide_kpm", Some(KPM_ARTIFACT));
    fs::write(&env.boot_id_path, "boot-2\n").unwrap();
    NativeBoot::start(env, 2, false);
    assert!(legacy.join("vpnhide_kpm/ctl.lock").exists());
    assert!(legacy.join("vpnhide_ports/observers.txt").exists());
    assert!(!legacy.join("vpnhide_ports/load_log").exists());
}

#[test]
fn kernel_config_dmesg_and_signature_helpers() {
    assert_eq!(kprobe_config(None), (None, None));
    assert_eq!(
        kprobe_config(Some("CONFIG_KPROBES=y\n# CONFIG_KRETPROBES is not set\n")),
        (Some(true), Some(false))
    );
    let raw: String = (0..60)
        .map(|i| format!("[{i}] vpnhide: line {i}\nnoise {i}\n"))
        .collect();
    let tail = dmesg_tail(&raw);
    assert_eq!(tail.len(), MAX_DMESG_TAIL);
    assert!(tail.last().unwrap().contains("line 59"));
    assert!(signature_rejected(129, "", &[]));
    assert!(signature_rejected(1, "Required key not available", &[]));
    assert!(!signature_rejected(1, "Invalid argument", &[]));
}

fn incompatible() -> KpmFact {
    KpmFact::Incompatible {
        found: Some("1.1.1".into()),
        detail: "the loaded vpnhide KPM is version 1.1.1; core needs 1.2.0 or newer".into(),
    }
}

#[test]
fn a_manually_loaded_kpm_is_a_normal_single_candidate() {
    // No module, nothing else: the live KPM is selected and adopted, never re-loaded.
    assert_eq!(
        decide(&facts(&[], KernelFact::Absent, KpmFact::Live)),
        Plan::Adopt(Kpm)
    );
}

#[test]
fn an_incompatible_live_kpm_is_a_typed_stop_whether_or_not_the_module_is_installed() {
    for installed in [&[][..], &[Kpm][..]] {
        assert_eq!(
            decide(&facts(installed, KernelFact::Absent, incompatible())),
            Plan::Stop(NativeBootOutcome::KpmIncompatible {
                found: Some("1.1.1".into()),
                minimum: vpnhide_activator::MIN_KPM_VERSION.into(),
                detail: "the loaded vpnhide KPM is version 1.1.1; core needs 1.2.0 or newer".into(),
            })
        );
    }
}

#[test]
fn a_live_manual_kpm_beside_another_backend_is_a_conflict() {
    for kpm in [KpmFact::Live, incompatible()] {
        for (installed, kernel, live) in [
            (&[Kmod][..], KernelFact::Absent, vec![Kpm]),
            (&[Zygisk][..], KernelFact::Absent, vec![Kpm]),
            (&[][..], KernelFact::Builtin, vec![Builtin, Kpm]),
            (&[][..], KernelFact::Kmod, vec![Kmod, Kpm]),
        ] {
            let plan = decide(&facts(installed, kernel.clone(), kpm.clone()));
            assert_eq!(
                plan,
                Plan::Stop(NativeBootOutcome::Conflict {
                    installed: installed.to_vec(),
                    live: live.clone(),
                    staged: vec![],
                }),
                "{installed:?} {kernel:?} {kpm:?}"
            );
        }
    }
}

#[test]
fn an_adopted_kpm_reports_the_filesystem_hiding_it_was_loaded_with() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    let fx = fake(&env, None);
    for fs_hiding in [Some(true), Some(false), None] {
        let mut f = gather(&env, KpmFact::Live);
        f.kpm_filesystem_hiding = fs_hiding;
        let boot = NativeBoot::decide_and_apply(env.clone(), 1, "boot-1", &f, None, false, &fx);
        assert_eq!(
            boot.status.outcome,
            NativeBootOutcome::Selected {
                backend: Kpm,
                mode: SelectionMode::Adopted,
                filesystem_hiding: fs_hiding,
            }
        );
        assert_eq!(boot.status.live, vec![Kpm]);
    }
}

#[test]
fn installed_obsolete_1x_modules_are_recorded_in_the_boot_status() {
    let dir = tempfile::tempdir().unwrap();
    let env = env(dir.path());
    install(dir.path(), "vpnhide_ports", None);
    install(dir.path(), "vpnhide_builtin", None);
    // Marked for removal: gone after the next reboot, so not reported.
    fs::write(dir.path().join("modules/vpnhide_builtin/remove"), "").unwrap();
    let fx = fake(&env, None);
    let f = gather(&env, KpmFact::NoRuntime);
    let boot = NativeBoot::decide_and_apply(env, 1, "boot-1", &f, None, false, &fx);
    assert_eq!(
        boot.status.obsolete_modules,
        vec!["vpnhide_ports".to_string()]
    );
    assert_eq!(boot.status.outcome, NativeBootOutcome::NoBackend);
}
