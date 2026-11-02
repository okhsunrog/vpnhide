//! Freeze startup observations and payload bytes before journaling any effect.
use super::*;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
mod bundled;
pub use bundled::{BundleIdentity, restore_environment};
pub(crate) fn validate_bundle_binding(binding: Option<&BundleIdentity>) -> Result<(), String> {
    bundled::validate_binding(binding)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadIdentity {
    pub source: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_contract: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionIntent {
    pub boot_id: String,
    pub filesystem_hiding: bool,
    pub action: TransitionAction,
    pub payload: Option<PayloadIdentity>,
    #[serde(default)]
    pub bundle: Option<BundleIdentity>,
    /// Publication only: this does not claim retirement of already-injected processes.
    pub retire_zygisk: Option<PathBuf>,
    pub retire_directory_identity: Option<(u64, u64)>,
}

impl TransitionIntent {
    pub fn possible_zygisk_exposure(&self) -> bool {
        matches!(
            self.action,
            TransitionAction::Adopt(NativeBackend::Zygisk)
                | TransitionAction::Load(NativeBackend::Zygisk)
        ) || self.retire_zygisk.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TransitionAction {
    Observe,
    Adopt(NativeBackend),
    Load(NativeBackend),
}

pub struct PreparedExecution {
    pub boot: NativeBoot,
    pub rejection: Option<super::evidence::RejectionProof>,
}

pub struct Prepared {
    pub intent: TransitionIntent,
    env: NativeEnv,
    facts: ScanFacts,
    plan: Plan,
    client: Option<CoreKpm>,
    staged: Option<File>,
    retire_target: Option<crate::zygisk::ZygiskTarget>,
    retire_bundled: bool,
    alternatives: std::collections::VecDeque<Prepared>,
}

impl Prepared {
    pub fn scan(
        env: NativeEnv,
        filesystem_hiding: bool,
        config: Option<&vpnhide_core_rpc::CanonicalConfig>,
    ) -> Result<Self, String> {
        let probe = probe_kpm(env.kpm_lock.as_deref());
        let mut facts = gather(&env, probe.fact);
        facts.kpm_filesystem_hiding = probe.filesystem_hiding;
        if let Some(root) = bundled::active_root(&env)? {
            return Self::from_bundle(
                env,
                filesystem_hiding,
                facts,
                probe.client,
                root,
                config.ok_or("canonical configuration required before bundled activation")?,
            );
        }
        Self::from_facts(env, filesystem_hiding, facts, probe.client)
    }

    pub fn from_facts(
        env: NativeEnv,
        filesystem_hiding: bool,
        facts: ScanFacts,
        client: Option<CoreKpm>,
    ) -> Result<Self, String> {
        let plan = decide(&facts);
        Self::from_plan(env, filesystem_hiding, facts, client, plan)
    }

    fn from_plan(
        env: NativeEnv,
        filesystem_hiding: bool,
        facts: ScanFacts,
        client: Option<CoreKpm>,
        plan: Plan,
    ) -> Result<Self, String> {
        let boot_id = read_boot_id(&env.boot_id_path);
        if boot_id.is_empty() || boot_id.len() > 128 {
            return Err("kernel boot identity unavailable".into());
        }
        let action = match plan {
            Plan::Stop(_) => TransitionAction::Observe,
            Plan::Adopt(b) => TransitionAction::Adopt(b),
            Plan::Load(b) => TransitionAction::Load(b),
        };
        let mut staged = None;
        let mut payload = None;
        if let TransitionAction::Load(backend) = action {
            let source = env.artifact(backend).ok_or("missing payload location")?;
            // Missing artifacts retain the established typed MissingArtifact outcome.
            if source.exists() {
                let parent = source.parent().ok_or("missing payload directory")?;
                let directory = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
                    .open(parent)
                    .map_err(|e| e.to_string())?;
                let name = source
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or("invalid artifact name")?;
                let mut input =
                    crate::payloads::open_beneath(&directory, name).map_err(|e| e.to_string())?;
                let staging = env
                    .state_file
                    .parent()
                    .ok_or("missing private owner directory")?;
                fs::create_dir_all(staging).map_err(|e| e.to_string())?;
                let mut output = tempfile::tempfile_in(staging).map_err(|e| e.to_string())?;
                let identity = copy_payload(&mut input, &mut output, source.clone())
                    .map_err(|e| e.to_string())?;
                payload = Some(identity);
                staged = Some(output);
            }
        }
        let retire_zygisk = (facts.installed.contains(&NativeBackend::Zygisk)
            && action != TransitionAction::Adopt(NativeBackend::Zygisk))
        .then(|| env.module_dir(NativeBackend::Zygisk))
        .flatten();
        let retire_target = retire_zygisk
            .as_ref()
            .and_then(|path| crate::zygisk::ZygiskTarget::open(path).ok());
        let retire_directory_identity = retire_target.as_ref().and_then(|t| t.identity().ok());
        let retire_bundled = retire_target
            .as_ref()
            .is_some_and(|t| t.is_bundled().unwrap_or(false));
        Ok(Self {
            intent: TransitionIntent {
                boot_id,
                filesystem_hiding,
                action,
                payload,
                bundle: None,
                retire_zygisk,
                retire_directory_identity,
            },
            env,
            facts,
            plan,
            client,
            staged,
            retire_target,
            retire_bundled,
            alternatives: std::collections::VecDeque::new(),
        })
    }

    pub(crate) fn into_candidates(mut self) -> std::collections::VecDeque<Self> {
        let mut candidates = std::mem::take(&mut self.alternatives);
        candidates.push_front(self);
        candidates
    }
    pub(crate) fn admit(
        self,
        journal: &crate::journal::Journal,
        explicit: bool,
    ) -> Result<Self, String> {
        let mut candidates = self.into_candidates();
        let mut skipped = false;
        while let Some(mut candidate) = candidates.pop_front() {
            journal.check_native_safety(&candidate.intent)?;
            if let TransitionAction::Load(backend) = candidate.intent.action
                && journal.backend_rejected(&candidate.intent.boot_id, backend)
            {
                let retained = journal
                    .native_startup
                    .iter()
                    .chain(journal.native_attempts.iter().map(|h| &h.attempt))
                    .filter(|a| a.boot_id == candidate.intent.boot_id)
                    .filter_map(|a| a.intent.as_ref())
                    .find(|i| i.action == candidate.intent.action);
                if retained.is_none_or(|i| i.bundle != candidate.intent.bundle) {
                    return Err("rejected candidate generation or policy changed; reboot before selecting another backend".into());
                }
                skipped = true;
                continue;
            }
            journal.check_native_attempt(&candidate.intent, explicit)?;
            if skipped
                && !super::evidence::ownership_clear(
                    candidate.environment(),
                    &candidate.intent.boot_id,
                )
            {
                return Err(
                    "ownership changed after the retained rejection; no successor dispatched"
                        .into(),
                );
            }
            candidate.alternatives = candidates;
            return Ok(candidate);
        }
        Err("all prepared candidates already rejected in this boot".into())
    }
    pub(crate) fn dispatch_ownership_clear(&self) -> bool {
        use crate::payloads::selection::Ownership;
        if self.intent.action == TransitionAction::Observe {
            return true;
        }
        if read_boot_id(&self.env.boot_id_path) != self.intent.boot_id {
            return false;
        }
        let probe = probe_kpm(self.env.kpm_lock.as_deref());
        let mut facts = gather(&self.env, probe.fact);
        if self.intent.bundle.is_none() {
            // Legacy installed files identify this exact prepared loader. Any
            // other installation or incompatible resident still blocks dispatch.
            facts.installed.retain(|b| Some(*b) != self.selected());
        }
        let observed = crate::payloads::device::resident_ownership(
            &facts,
            self.env
                .modules
                .as_deref()
                .is_some_and(|m| !obsolete_modules(m).is_empty()),
        );
        match self.intent.action {
            TransitionAction::Load(_) | TransitionAction::Adopt(NativeBackend::Zygisk) => {
                observed == Ownership::Clear && super::evidence::module_absent()
            }
            TransitionAction::Adopt(backend) => observed == Ownership::Resident(backend),
            TransitionAction::Observe => true,
        }
    }
    pub(crate) fn validate_binding(&self) -> Result<(), String> {
        bundled::validate_binding(self.intent.bundle.as_ref())
    }

    pub fn permit_recovery(&self, previous: &crate::journal::NativeStartup) -> Result<(), String> {
        if !previous.uncertain() {
            return Ok(());
        }
        if self.intent.boot_id == previous.boot_id {
            match self.intent.action {
                TransitionAction::Load(_) => return Err("startup load outcome is unknown in this boot; reboot, then use RepairActivation".into()),
                TransitionAction::Observe => return Err("startup effects cannot be reconciled in this boot; reboot, then use RepairActivation".into()),
                TransitionAction::Adopt(backend) => {
                    if previous.intent.as_ref().is_some_and(|i| matches!(i.action, TransitionAction::Load(old) | TransitionAction::Adopt(old) if old != backend)) {
                        return Err("live backend differs from interrupted intent; resolve ownership and reboot".into());
                    }
                }
            }
        }
        if let Plan::Stop(outcome) = &self.plan
            && !matches!(outcome, NativeBootOutcome::NoBackend)
        {
            return Err(format!("startup recovery preflight refused: {outcome:?}"));
        }
        Ok(())
    }

    pub(crate) fn bundled_retirement(&self) -> Option<&Path> {
        if self.retire_bundled {
            self.intent.retire_zygisk.as_deref()
        } else {
            None
        }
    }

    pub fn environment(&self) -> &NativeEnv {
        &self.env
    }
    pub fn projected_wire(&self) -> Option<&str> {
        self.intent.bundle.as_ref().map(|b| b.wire.as_str())
    }

    pub fn selected(&self) -> Option<NativeBackend> {
        match self.intent.action {
            TransitionAction::Load(b) | TransitionAction::Adopt(b) => Some(b),
            _ => None,
        }
    }

    /// No re-scan or best-effort persisted selection can redirect this prepared intent.
    pub fn execute(self, epoch: u64) -> NativeBoot {
        self.execute_detailed(epoch).boot
    }
    pub fn execute_detailed(self, epoch: u64) -> PreparedExecution {
        // Anonymous private inode: no crash leftovers, and child loaders access the
        // parent's retained descriptor rather than a replaceable module pathname.
        let staged_path = self
            .staged
            .as_ref()
            .map(|f| PathBuf::from(format!("/proc/{}/fd/{}", std::process::id(), f.as_raw_fd())));
        let effects = PinnedEffects {
            source: self.intent.payload.as_ref().map(|p| p.source.as_path()),
            staged: staged_path.as_deref(),
            retire_path: self.intent.retire_zygisk.as_deref(),
            retire_target: self.retire_target.as_ref(),
            retire_bundled: self.retire_bundled,
            failure: std::cell::RefCell::new(None),
        };
        crate::recovery::maybe_crash_for_test("startup-before-effects");
        let boot = NativeBoot::apply_prepared_plan(
            self.env,
            epoch,
            &self.intent.boot_id,
            &self.facts,
            self.client,
            self.intent.filesystem_hiding,
            &effects,
            self.plan,
        );
        crate::recovery::maybe_crash_for_test("startup-after-effects");
        boot.persist();
        let rejection = super::evidence::rejection(&self.intent, &boot, *effects.failure.borrow());
        PreparedExecution { boot, rejection }
    }
}

fn copy_payload(
    input: &mut File,
    output: &mut File,
    source: PathBuf,
) -> io::Result<PayloadIdentity> {
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        if bytes > crate::payloads::MAX_PAYLOAD_BYTES {
            return Err(io::Error::other("payload exceeds preparation budget"));
        }
        output.write_all(&buffer[..n])?;
        hash.update(&buffer[..n]);
    }
    if bytes == 0 {
        return Err(io::Error::other("empty payload"));
    }
    output.sync_all()?;
    Ok(PayloadIdentity {
        source,
        sha256: format!("{:x}", hash.finalize()),
        bytes,
        init_contract: None,
    })
}

struct PinnedEffects<'a> {
    source: Option<&'a Path>,
    staged: Option<&'a Path>,
    retire_path: Option<&'a Path>,
    retire_target: Option<&'a crate::zygisk::ZygiskTarget>,
    retire_bundled: bool,
    failure: std::cell::RefCell<Option<super::evidence::CapturedFailure>>,
}
impl PinnedEffects<'_> {
    fn path(&self, source: &Path) -> Result<&Path, String> {
        if self.source != Some(source) {
            return Err("load differs from prepared payload intent".into());
        }
        self.staged
            .ok_or_else(|| "prepared payload unavailable".into())
    }
}
impl Effects for PinnedEffects<'_> {
    fn insmod(&self, ctl: &Path, artifact: &Path, fs: bool) -> Result<(), KmodLoadFailure> {
        let result = self
            .path(artifact)
            .map_err(|stderr| KmodLoadFailure {
                origin: vpnhide_activator::KmodLoadOrigin::NotDispatched,
                exit: 127,
                stderr,
            })
            .and_then(|artifact| SystemEffects.insmod(ctl, artifact, fs));
        if let Err(failure) = &result {
            *self.failure.borrow_mut() = Some(super::evidence::CapturedFailure::Kmod {
                origin: failure.origin,
                code: failure.exit,
            });
        }
        result
    }
    fn kpm_load(
        &self,
        kpm: &CoreKpm,
        ctl: &Path,
        artifact: &Path,
        fs: bool,
    ) -> Result<(), KpmLoadFailure> {
        let result = self
            .path(artifact)
            .map_err(KpmLoadFailure::BeforeDispatch)
            .and_then(|artifact| SystemEffects.kpm_load(kpm, ctl, artifact, fs));
        if let Err(failure) = &result {
            *self.failure.borrow_mut() = Some(match failure {
                KpmLoadFailure::BeforeDispatch(_) => {
                    super::evidence::CapturedFailure::KpmBeforeDispatch
                }
                KpmLoadFailure::AfterDispatch(_) => {
                    super::evidence::CapturedFailure::KpmAfterDispatch
                }
            });
        }
        result
    }
    fn retire_zygisk(&self, dir: &Path) -> Result<(), String> {
        if self.retire_path != Some(dir) {
            return Err("retirement differs from prepared intent".into());
        }
        if self.retire_bundled {
            let target = crate::zygisk::ZygiskTarget::for_revocation(dir)?;
            if target.identity()?
                != self
                    .retire_target
                    .ok_or("missing pinned recipient")?
                    .identity()?
            {
                return Err("retirement recipient replaced".into());
            }
            return if target.authorization_absent()? {
                Ok(())
            } else {
                Err("authorization was not journaled as revoked".into())
            };
        }
        retire_zygisk_target(
            self.retire_target
                .ok_or("Zygisk recipient could not be pinned")?,
        )
    }
    fn kpm_kernel(&self) -> Result<(String, bool), String> {
        SystemEffects.kpm_kernel()
    }
    fn kprobes(&self) -> (Option<bool>, Option<bool>) {
        SystemEffects.kprobes()
    }
    fn dmesg_tail(&self) -> Vec<String> {
        SystemEffects.dmesg_tail()
    }
    fn uname(&self) -> String {
        SystemEffects.uname()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{Journal, NativeStartup};
    use std::io::{Seek, SeekFrom};
    use std::os::unix::fs::symlink;

    fn setup(
        root: &Path,
        boot: &str,
        installed: Vec<NativeBackend>,
        kernel: KernelFact,
    ) -> Prepared {
        let env = NativeEnv {
            ctl: root.join("ctl"),
            modules: Some(root.join("modules")),
            kpm_lock: None,
            zygisk_module: None,
            boot_id_path: root.join("boot-id"),
            state_file: root.join("owner/native.json"),
            legacy_root: None,
            apatch_kpms: root.join("kpms"),
            payload_override: None,
            bundle_identity: None,
            loader: None,
        };
        fs::write(&env.boot_id_path, boot).unwrap();
        Prepared::from_facts(
            env,
            false,
            ScanFacts {
                installed,
                staged: Vec::new(),
                kernel,
                kpm: KpmFact::NoRuntime,
                kpm_may_be_live: false,
                kpm_filesystem_hiding: None,
            },
            None,
        )
        .unwrap()
    }
    fn artifact(root: &Path) -> PathBuf {
        let path = root.join("modules/vpnhide_kmod/vpnhide_kmod.ko");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"original payload").unwrap();
        path
    }
    fn attempt(intent: TransitionIntent) -> NativeStartup {
        NativeStartup {
            boot_id: intent.boot_id.clone(),
            config_revision: 3,
            filesystem_hiding: intent.filesystem_hiding,
            intent: Some(intent),
            repair_op: None,
            completed: None,
        }
    }

    #[test]
    fn execution_does_not_reselect_from_diagnostic_facts() {
        let root = tempfile::tempdir().unwrap();
        let mut prepared = setup(root.path(), "boot-a", Vec::new(), KernelFact::Absent);
        assert_eq!(prepared.intent.action, TransitionAction::Observe);
        // Diagnostic inputs are not authority to replace a frozen decision.
        // This intentional internal mutation would have selected Builtin when
        // execute() called decide(facts) a second time.
        prepared.facts.kernel = KernelFact::Builtin;
        let boot = prepared.execute(1);
        assert_eq!(boot.selected(), None);
        assert_eq!(boot.status.outcome, NativeBootOutcome::NoBackend);
    }

    #[test]
    fn payload_is_an_anonymous_exact_snapshot_not_a_late_path_lookup() {
        let root = tempfile::tempdir().unwrap();
        let path = artifact(root.path());
        let mut prepared = setup(
            root.path(),
            "boot-a",
            vec![NativeBackend::Kmod],
            KernelFact::Absent,
        );
        let proof = prepared.intent.payload.as_ref().unwrap();
        assert_eq!(proof.source, path);
        assert_eq!(
            proof.sha256,
            format!("{:x}", Sha256::digest(b"original payload"))
        );
        assert_eq!(proof.bytes, 16);
        fs::write(&path, b"replaced payload").unwrap();
        let file = prepared.staged.as_mut().unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"original payload");
        assert_eq!(
            fs::read_dir(root.path().join("owner")).unwrap().count(),
            0,
            "no named payload survives a crash"
        );
    }

    #[test]
    fn symlink_payload_is_never_prepared() {
        let root = tempfile::tempdir().unwrap();
        let path = artifact(root.path());
        fs::remove_file(&path).unwrap();
        symlink("/etc/passwd", &path).unwrap();
        let mut prepared = setup(root.path(), "boot-a", Vec::new(), KernelFact::Absent);
        prepared.facts.installed = vec![NativeBackend::Kmod];
        assert!(Prepared::from_facts(prepared.env, false, prepared.facts, None).is_err());
    }

    #[test]
    fn recovery_requires_reboot_for_a_possibly_executed_load_not_for_live_adoption() {
        let root = tempfile::tempdir().unwrap();
        artifact(root.path());
        let current = setup(
            root.path(),
            "boot-a",
            vec![NativeBackend::Kmod],
            KernelFact::Absent,
        );
        let old = attempt(current.intent.clone());
        assert!(current.permit_recovery(&old).is_err());
        let live = setup(
            root.path(),
            "boot-a",
            vec![NativeBackend::Kmod],
            KernelFact::Kmod,
        );
        assert!(live.permit_recovery(&old).is_ok());
        let different = setup(root.path(), "boot-a", Vec::new(), KernelFact::Builtin);
        assert!(different.permit_recovery(&old).is_err());
        let next_boot = setup(
            root.path(),
            "boot-b",
            vec![NativeBackend::Kmod],
            KernelFact::Absent,
        );
        assert!(next_boot.permit_recovery(&old).is_ok());
        let absent = setup(root.path(), "boot-a", Vec::new(), KernelFact::Absent);
        assert!(absent.permit_recovery(&old).is_err());
        let absent_after_boot = setup(root.path(), "boot-b", Vec::new(), KernelFact::Absent);
        assert!(absent_after_boot.permit_recovery(&old).is_ok());
    }

    #[test]
    fn retiring_an_unknown_legacy_zygisk_also_fences_kernel_activation_for_the_boot() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = Journal::default();
        let conflict = setup(
            root.path(),
            "boot-a",
            vec![NativeBackend::Zygisk, NativeBackend::Kpm],
            KernelFact::Absent,
        );
        assert!(matches!(conflict.intent.action, TransitionAction::Observe));
        assert!(conflict.intent.retire_zygisk.is_some());
        journal.replace_native_startup(attempt(conflict.intent));
        let kernel = setup(root.path(), "boot-a", vec![], KernelFact::Builtin);
        assert!(journal.check_native_attempt(&kernel.intent, true).is_err());
        let mut old = serde_json::to_value(&journal).unwrap();
        old["version"] = 8.into();
        old["zygisk_exposure_boot"] = "prior-boot".into();
        old.as_object_mut().unwrap().remove("zygisk_exposure_boots");
        let path = root.path().join("journal.json");
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let migrated = crate::journal::read(&path).unwrap();
        assert_eq!(migrated.zygisk_exposure_boot.as_deref(), Some("prior-boot"));
        assert!(migrated.zygisk_exposed_in("boot-a"));
        assert!(
            migrated
                .check_native_attempt(&kernel.intent, false)
                .is_err()
        );
        let reboot = setup(root.path(), "boot-b", vec![], KernelFact::Builtin);
        assert!(migrated.check_native_attempt(&reboot.intent, true).is_ok());
    }

    #[test]
    fn zygisk_exposure_survives_history_pruning_and_requires_a_new_boot() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = Journal::default();
        let zygisk = setup(
            root.path(),
            "boot-a",
            vec![NativeBackend::Zygisk],
            KernelFact::Absent,
        );
        journal.replace_native_startup(attempt(zygisk.intent));
        for _ in 0..32 {
            let observed = setup(root.path(), "boot-a", Vec::new(), KernelFact::Absent);
            journal.replace_native_startup(attempt(observed.intent));
        }
        let path = root.path().join("journal.json");
        crate::journal::write_atomic(&path, &journal).unwrap();
        let restored = crate::journal::read(&path).unwrap();
        assert_eq!(restored.zygisk_exposure_boot.as_deref(), Some("boot-a"));
        let kernel = setup(root.path(), "boot-a", Vec::new(), KernelFact::Builtin);
        assert!(
            restored
                .check_native_attempt(&kernel.intent, false)
                .is_err()
        );
        assert!(restored.check_native_attempt(&kernel.intent, true).is_err());
        let after_reboot = setup(root.path(), "boot-b", Vec::new(), KernelFact::Builtin);
        assert!(
            restored
                .check_native_attempt(&after_reboot.intent, false)
                .is_ok()
        );
        let zygisk = setup(
            root.path(),
            "boot-a",
            vec![NativeBackend::Zygisk],
            KernelFact::Absent,
        );
        assert!(restored.check_native_attempt(&zygisk.intent, true).is_ok());
    }

    #[test]
    fn load_budget_and_quarantine_outlive_bounded_attempt_history() {
        let root = tempfile::tempdir().unwrap();
        artifact(root.path());
        let prepared = setup(
            root.path(),
            "boot-a",
            vec![NativeBackend::Kmod],
            KernelFact::Absent,
        );
        let mut journal = Journal::default();
        journal
            .check_native_attempt(&prepared.intent, false)
            .unwrap();
        journal.replace_native_startup(attempt(prepared.intent.clone()));
        for _ in 0..32 {
            let observe = setup(root.path(), "boot-a", Vec::new(), KernelFact::Absent);
            journal.replace_native_startup(attempt(observe.intent));
        }
        assert_eq!(
            journal.native_attempts.len(),
            crate::journal::MAX_NATIVE_ATTEMPTS
        );
        assert!(
            journal
                .check_native_attempt(&prepared.intent, true)
                .is_err()
        );
        let next_boot = setup(
            root.path(),
            "boot-b",
            vec![NativeBackend::Kmod],
            KernelFact::Absent,
        );
        assert!(
            journal
                .check_native_attempt(&next_boot.intent, false)
                .is_err()
        );
        assert!(
            journal
                .check_native_attempt(&next_boot.intent, true)
                .is_ok()
        );
        let mut changed = next_boot.intent.clone();
        changed.payload.as_mut().unwrap().sha256 = "c".repeat(64);
        assert!(journal.check_native_attempt(&changed, false).is_ok());
    }
}
