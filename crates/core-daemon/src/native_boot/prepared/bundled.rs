//! The production catalog-to-prepared boundary. No loader or policy writes occur here.
use super::*;
use crate::payloads::{
    Catalog, Manifest,
    selection::{self, Capabilities, Decision, Policy, Runtime, Support},
};
use vpnhide_activator::PackageUidMap;
use vpnhide_core_rpc::CanonicalConfig;
use vpnhide_protocol::hook_ids::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleIdentity {
    pub root: PathBuf,
    pub manifest_sha256: String,
    pub source_commit: String,
    pub wire: String,
}

fn manifest_digest(manifest: &Manifest) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(manifest).expect("manifest serialization"))
    )
}

pub(super) fn active_root(env: &NativeEnv) -> Result<Option<PathBuf>, String> {
    let Some(modules) = &env.modules else {
        return Ok(None);
    };
    let root = modules.join("vpnhide_core");
    match fs::symlink_metadata(root.join("payloads/manifest.json")) {
        Ok(_) => Ok(Some(root)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot inspect active payload catalog: {e}")),
    }
}

impl Prepared {
    pub(super) fn from_bundle(
        env: NativeEnv,
        filesystem_hiding: bool,
        facts: ScanFacts,
        client: Option<CoreKpm>,
        root: PathBuf,
        config: &CanonicalConfig,
    ) -> Result<Self, String> {
        let catalog = verified_catalog(&root)?;
        catalog
            .manifest
            .require_complete()
            .map_err(|e| e.to_string())?;
        let packages =
            PackageUidMap::from_pm().map_err(|e| format!("package inventory unavailable: {e}"))?;
        let mut capabilities = crate::payloads::device::observe(&env, &facts);
        // Availability comes from the public-API companion and actual mapped
        // bytes, followed by real app specialization. Provider names cannot
        // promote this evidence; hook health remains a separate measurement.
        capabilities.zygisk = if env.loader.as_ref().is_some_and(|loader| {
            loader.available(&catalog, capabilities.abi, capabilities.needs_arm32_zygisk)
        }) {
            Runtime::Ready
        } else {
            Runtime::Unknown
        };
        let mut support = vec![
            (
                NativeBackend::Kmod,
                compiled(KERNEL_HOOK_MASK | KMOD_HOOK_MASK, 65536, true),
            ),
            (
                NativeBackend::Kpm,
                compiled(
                    KERNEL_HOOK_MASK | KPM_HOOK_MASK,
                    (vpnhide_protocol::KPM_ARGS_LEN - 1) as u32,
                    true,
                ),
            ),
            (
                NativeBackend::Zygisk,
                compiled(ZYGISK_HOOK_MASK, 65536, false),
            ),
        ];
        if let Ok(status) = crate::activation::read_status(&env.ctl) {
            let backend = match facts.kernel {
                KernelFact::Builtin => Some(NativeBackend::Builtin),
                KernelFact::Kmod => Some(NativeBackend::Kmod),
                _ => None,
            };
            if let Some(backend) = backend {
                support.retain(|(b, _)| *b != backend);
                support.push((
                    backend,
                    compiled(
                        if status.error == 0 { status.hooks } else { 0 },
                        65536,
                        true,
                    ),
                ));
            }
        }
        if facts.kpm == KpmFact::Live {
            support.retain(|(b, _)| *b != NativeBackend::Kpm);
            if let Some(status) = client.as_ref().and_then(|c| c.status().ok().flatten()) {
                support.push((
                    NativeBackend::Kpm,
                    compiled(
                        if status.error == 0 { status.hooks } else { 0 },
                        (vpnhide_protocol::KPM_ARGS_LEN - 1) as u32,
                        true,
                    ),
                ));
            }
        }
        Self::bind_bundle(
            env,
            filesystem_hiding,
            facts,
            client,
            root,
            catalog,
            config,
            &packages,
            &capabilities,
            &support,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_bundle(
        mut env: NativeEnv,
        filesystem_hiding: bool,
        facts: ScanFacts,
        client: Option<CoreKpm>,
        root: PathBuf,
        catalog: Catalog,
        config: &CanonicalConfig,
        packages: &PackageUidMap,
        capabilities: &Capabilities,
        support: &[(NativeBackend, Support)],
    ) -> Result<Self, String> {
        let mut projections = Vec::new();
        for backend in [
            NativeBackend::Builtin,
            NativeBackend::Kmod,
            NativeBackend::Kpm,
            NativeBackend::Zygisk,
        ] {
            if let Ok(projected) = crate::payloads::projection::project(config, backend, packages) {
                projections.push((backend, projected));
            }
        }
        let requirements: Vec<(NativeBackend, Policy)> = projections
            .iter()
            .map(|(b, p)| (*b, p.requirements))
            .collect();
        let assessment =
            selection::assess_projected(&catalog.manifest, capabilities, &requirements, support)
                .map_err(|e| e.to_string())?;
        let (backend, resident) = match assessment.decision {
            Decision::Select(b) => (b, false),
            Decision::ReconcileResident(b)
                if crate::payloads::device::resident_eligible(&assessment, b) =>
            {
                (b, true)
            }
            _ => {
                return Err(format!(
                    "bundled selection held: {:?}; candidates={:?}",
                    assessment.decision, assessment.candidates
                ));
            }
        };
        let projected = projections
            .into_iter()
            .find(|(b, _)| *b == backend)
            .ok_or("selected projection missing")?
            .1;
        let candidate = assessment
            .candidates
            .iter()
            .find(|c| c.backend == backend)
            .ok_or("selected candidate missing")?;
        let load = !resident && matches!(backend, NativeBackend::Kmod | NativeBackend::Kpm);
        let expected = if load {
            let index = *candidate
                .payload_indices
                .first()
                .ok_or("selected payload missing")?;
            let metadata = &catalog.payloads[index].metadata;
            env.payload_override = Some(
                root.join(
                    metadata
                        .implementation
                        .relative_path()
                        .map_err(|e| e.to_string())?,
                ),
            );
            Some(metadata.clone())
        } else {
            None
        };
        if backend == NativeBackend::Zygisk {
            env.zygisk_module = Some(root.clone());
        }
        let plan = if load {
            Plan::Load(backend)
        } else {
            Plan::Adopt(backend)
        };
        let mut prepared = Self::from_plan(env, filesystem_hiding, facts, client, plan)?;
        if let Some(expected) = expected {
            let copied = prepared
                .intent
                .payload
                .as_ref()
                .ok_or("verified payload vanished before preparation")?;
            if copied.sha256 != expected.sha256 || copied.bytes != expected.bytes {
                return Err(
                    "payload changed after catalog verification; no effect dispatched".into(),
                );
            }
        }
        prepared.intent.bundle = Some(BundleIdentity {
            root,
            manifest_sha256: manifest_digest(&catalog.manifest),
            source_commit: catalog.manifest.bundle_commit,
            wire: projected.wire,
        });
        Ok(prepared)
    }
}

fn compiled(hooks: u32, wire_capacity: u32, default_mask: bool) -> Support {
    Support {
        hooks,
        target_capacity: vpnhide_protocol::MAX_TARGET_UIDS as u32,
        wire_capacity,
        default_mask,
    }
}

/// Rebind only the same active generation. A staged/replaced bundle cannot change
/// the recipient or pretend it is the payload retained by an earlier startup.
pub fn restore_environment(
    mut env: NativeEnv,
    intent: Option<&TransitionIntent>,
) -> Result<NativeEnv, String> {
    let active = active_root(&env)?;
    let Some(binding) = intent.and_then(|i| i.bundle.as_ref()) else {
        if active.is_some() {
            return Err(
                "active bundle has no matching durable generation; explicit recovery required"
                    .into(),
            );
        }
        return Ok(env);
    };
    if active.as_ref() != Some(&binding.root) {
        return Err("active bundle root changed".into());
    }
    let catalog = verified_catalog(&binding.root)?;
    catalog
        .manifest
        .require_complete()
        .map_err(|e| e.to_string())?;
    if manifest_digest(&catalog.manifest) != binding.manifest_sha256
        || catalog.manifest.bundle_commit != binding.source_commit
    {
        return Err("active bundle generation changed in this boot; no startup replayed".into());
    }
    let intent = intent.expect("binding came from intent");
    if let Some(payload) = &intent.payload {
        let matches = catalog.payloads.iter().any(|p| {
            p.metadata.sha256 == payload.sha256
                && p.metadata.bytes == payload.bytes
                && p.metadata
                    .implementation
                    .relative_path()
                    .is_ok_and(|path| binding.root.join(path) == payload.source)
        });
        if !matches {
            return Err("prepared payload is not a member of the active catalog".into());
        }
        env.payload_override = Some(payload.source.clone());
    }
    if intent.action == TransitionAction::Adopt(NativeBackend::Zygisk) {
        env.zygisk_module = Some(binding.root.clone());
    }
    Ok(env)
}

fn verified_catalog(root: &Path) -> Result<Catalog, String> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(root)
        .map_err(|e| e.to_string())?;
    let mut prop = String::new();
    crate::payloads::open_beneath(&directory, "module.prop")
        .map_err(|e| e.to_string())?
        .take(8193)
        .read_to_string(&mut prop)
        .map_err(|e| e.to_string())?;
    let ids: Vec<_> = prop
        .lines()
        .filter_map(|line| line.strip_prefix("id="))
        .collect();
    if prop.len() > 8192
        || ids != ["vpnhide_core"]
        || root.join("disable").exists()
        || root.join("remove").exists()
    {
        return Err("active Core module identity/eligibility invalid".into());
    }
    Catalog::open(root).map_err(|e| format!("active catalog: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payloads::{Abi, KMIS, Payload, PayloadKind, selection::Ownership};
    use std::io::{Seek, SeekFrom};
    use vpnhide_core_rpc::{AppConfig, NativeRole};

    fn fixture() -> (
        tempfile::TempDir,
        NativeEnv,
        PathBuf,
        CanonicalConfig,
        PackageUidMap,
        Capabilities,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let modules = temp.path().join("modules");
        let root = modules.join("vpnhide_core");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("module.prop"), "id=vpnhide_core\n").unwrap();
        let kinds = KMIS
            .iter()
            .map(|kmi| PayloadKind::Kmod { kmi: (*kmi).into() })
            .chain([
                PayloadKind::Kpm,
                PayloadKind::Zygisk { abi: Abi::Arm64 },
                PayloadKind::Zygisk { abi: Abi::Arm32 },
            ]);
        let payloads = kinds
            .map(|kind| {
                let path = root.join(kind.relative_path().unwrap());
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"payload").unwrap();
                Payload {
                    implementation: kind,
                    version: "test".into(),
                    source_commit: "a".repeat(40),
                    sha256: format!("{:x}", Sha256::digest(b"payload")),
                    bytes: 7,
                    control_version: 2,
                    preflight_protocol: None,
                }
            })
            .collect();
        let manifest = Manifest {
            schema: 1,
            bundle_commit: "b".repeat(40),
            payloads,
        };
        fs::write(
            root.join("payloads/manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let boot_id_path = temp.path().join("boot");
        fs::write(&boot_id_path, "boot-a").unwrap();
        let env = NativeEnv {
            ctl: temp.path().join("ctl"),
            modules: Some(modules),
            kpm_lock: None,
            zygisk_module: None,
            boot_id_path,
            state_file: temp.path().join("owner/native.json"),
            legacy_root: None,
            apatch_kpms: temp.path().join("kpms"),
            payload_override: None,
            loader: None,
        };
        let mut config = CanonicalConfig::default();
        config.apps.insert(
            "com.example".into(),
            AppConfig {
                native: NativeRole {
                    enabled: true,
                    kernel: Some(vec!["dev_ioctl".into()]),
                    zygisk: Some(vec!["zygisk_ioctl".into()]),
                },
                ..Default::default()
            },
        );
        let packages =
            vpnhide_activator::parse_pm_packages("package:com.example uid:10123,110123\n");
        let caps = Capabilities {
            abi: Abi::Arm64,
            kernel_release: "6.1.157-android14".into(),
            builtin: Runtime::Absent,
            module_loading: Runtime::Ready,
            kpm: Runtime::Absent,
            zygisk: Runtime::Unknown,
            needs_arm32_zygisk: true,
            ownership: Ownership::Clear,
        };
        (temp, env, root, config, packages, caps)
    }
    fn facts() -> ScanFacts {
        ScanFacts {
            installed: vec![],
            staged: vec![],
            kernel: KernelFact::Absent,
            kpm: KpmFact::NoRuntime,
            kpm_may_be_live: false,
            kpm_filesystem_hiding: None,
        }
    }
    fn support() -> Vec<(NativeBackend, Support)> {
        vec![(NativeBackend::Kmod, compiled(KERNEL_HOOK_MASK, 65536, true))]
    }

    #[test]
    fn catalog_policy_and_exact_payload_are_frozen_together() {
        let (_temp, env, root, config, packages, caps) = fixture();
        let catalog = verified_catalog(&root).unwrap();
        let mut prepared = Prepared::bind_bundle(
            env.clone(),
            false,
            facts(),
            None,
            root.clone(),
            catalog,
            &config,
            &packages,
            &caps,
            &support(),
        )
        .unwrap();
        assert_eq!(prepared.selected(), Some(NativeBackend::Kmod));
        assert_eq!(
            prepared.intent.bundle.as_ref().unwrap().source_commit,
            "b".repeat(40)
        );
        let wire = prepared.projected_wire().unwrap();
        assert_eq!(
            vpnhide_protocol::parse_config(wire.as_bytes())
                .unwrap()
                .targets
                .iter()
                .map(|t| t.uid)
                .collect::<Vec<_>>(),
            vec![10123, 110123]
        );
        let source = prepared.intent.payload.as_ref().unwrap().source.clone();
        fs::write(&source, b"replacement").unwrap();
        let staged = prepared.staged.as_mut().unwrap();
        staged.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        staged.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"payload");
        assert!(restore_environment(env, Some(&prepared.intent)).is_err());
    }

    #[test]
    fn mutation_between_verification_and_preparation_is_not_loaded() {
        let (_temp, env, root, config, packages, caps) = fixture();
        let catalog = verified_catalog(&root).unwrap();
        let chosen = catalog
            .manifest
            .payloads
            .iter()
            .find(|p| matches!(&p.implementation,PayloadKind::Kmod{kmi} if kmi=="android14-6.1"))
            .unwrap();
        fs::write(
            root.join(chosen.implementation.relative_path().unwrap()),
            b"changed",
        )
        .unwrap();
        let result = Prepared::bind_bundle(
            env,
            false,
            facts(),
            None,
            root,
            catalog,
            &config,
            &packages,
            &caps,
            &support(),
        );
        assert!(
            result
                .err()
                .unwrap()
                .contains("changed after catalog verification")
        );
    }

    #[test]
    fn unknown_kmi_and_missing_policy_coverage_are_not_arbitrary_choices() {
        let (_temp, env, root, config, packages, mut caps) = fixture();
        caps.kernel_release.clear();
        assert!(
            Prepared::bind_bundle(
                env.clone(),
                false,
                facts(),
                None,
                root.clone(),
                verified_catalog(&root).unwrap(),
                &config,
                &packages,
                &caps,
                &support()
            )
            .is_err()
        );
        caps.kernel_release = "6.1.157-android14".into();
        assert!(
            Prepared::bind_bundle(
                env,
                false,
                facts(),
                None,
                root.clone(),
                verified_catalog(&root).unwrap(),
                &config,
                &packages,
                &caps,
                &[(NativeBackend::Kmod, compiled(0, 65536, true))]
            )
            .is_err()
        );
    }

    #[test]
    fn restore_requires_the_same_active_manifest_and_does_not_reselect() {
        let (_temp, env, root, config, packages, caps) = fixture();
        let prepared = Prepared::bind_bundle(
            env.clone(),
            false,
            facts(),
            None,
            root.clone(),
            verified_catalog(&root).unwrap(),
            &config,
            &packages,
            &caps,
            &support(),
        )
        .unwrap();
        assert!(restore_environment(env.clone(), Some(&prepared.intent)).is_ok());
        assert!(restore_environment(env.clone(), None).is_err());
        fs::write(root.join("module.prop"), "id=other_module\n").unwrap();
        assert!(restore_environment(env.clone(), Some(&prepared.intent)).is_err());
        fs::write(root.join("module.prop"), "id=vpnhide_core\n").unwrap();
        fs::write(root.join("disable"), "").unwrap();
        assert!(restore_environment(env.clone(), Some(&prepared.intent)).is_err());
        fs::remove_file(root.join("disable")).unwrap();
        let mut manifest = verified_catalog(&root).unwrap().manifest;
        manifest.bundle_commit = "c".repeat(40);
        fs::write(
            root.join("payloads/manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            restore_environment(env, Some(&prepared.intent))
                .err()
                .unwrap()
                .contains("generation changed")
        );
    }
}
