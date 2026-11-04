//! Pure eligibility and deterministic selection. No filesystem, runtime probes or effects.
//! Probe adapters must supply positive evidence; manager names are not capabilities.

use vpnhide_core_rpc::NativeBackend;

use super::{Abi, Manifest, PayloadKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Ready,
    Absent,
    AuthenticationRequired,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    /// Verified no residual hooks that conflict with a new activation.
    Clear,
    /// Existing effect must be reconciled/retired by the lifecycle, not this selector.
    Resident(NativeBackend),
    Foreign,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Capabilities {
    pub abi: Abi,
    pub kernel_release: String,
    pub builtin: Runtime,
    pub module_loading: Runtime,
    pub kpm: Runtime,
    pub zygisk: Runtime,
    pub needs_arm32_zygisk: bool,
    pub ownership: Ownership,
}

/// Full resolved policy requirements, including every profile and the core self target.
/// Resolution failure must not be represented as an empty policy.
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub resolved: bool,
    pub targets: u32,
    pub wire_bytes: u32,
    pub default_mask: u32,
    pub required_hooks: u32,
}

/// Evidence for this exact policy/backend, produced by projection and capability adapters.
/// `None` in a candidate is unknown, never inferred from the package release number.
#[derive(Debug, Clone, Copy)]
pub struct Support {
    pub hooks: u32,
    pub target_capacity: u32,
    pub wire_capacity: u32,
    pub default_mask: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    MissingPayload,
    WrongArchitecture,
    UnknownKmi,
    NoMatchingKmi,
    AmbiguousKmi,
    ConflictingKmiEvidence,
    RuntimeAbsent,
    AuthenticationRequired,
    UnsupportedRuntime,
    RuntimeUnknown,
    PolicyUnresolved,
    CoverageUnknown,
    RequiredHooksMissing,
    TargetCapacity,
    WireCapacity,
    DefaultMaskUnsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub backend: NativeBackend,
    /// All payloads necessary for the candidate, including 32-bit Zygisk on 64-bit devices.
    pub payload_indices: Vec<usize>,
    pub eligibility: Result<(), Rejection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Select(NativeBackend),
    ReconcileResident(NativeBackend),
    BlockedOwnership,
    NoEligibleCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    pub kmi: super::kmi::KmiResolution,
    pub candidates: Vec<Candidate>,
    pub decision: Decision,
}

/// Select only on clean ownership. Residency is not proof of health or permission to
/// adopt; a separate journaled lifecycle reconciles it before permitting another load.
/// The manifest must be validated before calling; invalid manifests yield an error.
pub fn assess(
    manifest: &Manifest,
    device: &Capabilities,
    policy: Policy,
    support: &[(NativeBackend, Support)],
) -> std::io::Result<Assessment> {
    let requirements: Vec<_> = [
        NativeBackend::Builtin,
        NativeBackend::Kmod,
        NativeBackend::Kpm,
        NativeBackend::Zygisk,
    ]
    .into_iter()
    .map(|backend| (backend, policy))
    .collect();
    assess_projected(manifest, device, &requirements, support)
}

/// Candidate requirements must come from complete backend-specific projections of
/// the same canonical configuration and package inventory. Kernel and libc hook
/// IDs are different; absence or duplicate projections are not an empty policy.
pub fn assess_projected(
    manifest: &Manifest,
    device: &Capabilities,
    requirements: &[(NativeBackend, Policy)],
    support: &[(NativeBackend, Support)],
) -> std::io::Result<Assessment> {
    manifest.validate()?;
    let kmi = super::kmi::resolve(manifest, &device.kernel_release)?;
    let mut candidates = Vec::new();
    for backend in [
        NativeBackend::Builtin,
        NativeBackend::Kmod,
        NativeBackend::Kpm,
        NativeBackend::Zygisk,
    ] {
        let (indices, availability) = available(manifest, device, &kmi, backend);
        let runtime = match backend {
            NativeBackend::Builtin => device.builtin,
            NativeBackend::Kmod => device.module_loading,
            NativeBackend::Kpm => device.kpm,
            NativeBackend::Zygisk => device.zygisk,
        };
        let mut projected = requirements.iter().filter(|(b, _)| *b == backend);
        let policy = projected
            .next()
            .map(|(_, policy)| policy)
            .filter(|_| projected.next().is_none());
        let eligibility = availability
            .and_then(|()| runtime_ready(runtime))
            .and_then(|()| {
                let policy = policy.ok_or(Rejection::PolicyUnresolved)?;
                if !policy.resolved {
                    return Err(Rejection::PolicyUnresolved);
                }
                let mut entries = support.iter().filter(|(b, _)| *b == backend);
                let Some((_, evidence)) = entries.next() else {
                    return Err(Rejection::CoverageUnknown);
                };
                // Conflicting evidence must not be resolved by input order.
                if entries.next().is_some() {
                    return Err(Rejection::CoverageUnknown);
                }
                if policy.default_mask != 0
                    && (!evidence.default_mask || backend == NativeBackend::Zygisk)
                {
                    return Err(Rejection::DefaultMaskUnsupported);
                }
                if policy.targets > evidence.target_capacity {
                    return Err(Rejection::TargetCapacity);
                }
                if policy.wire_bytes > evidence.wire_capacity {
                    return Err(Rejection::WireCapacity);
                }
                if policy.required_hooks & !evidence.hooks != 0 {
                    return Err(Rejection::RequiredHooksMissing);
                }
                Ok(())
            });
        candidates.push(Candidate {
            backend,
            payload_indices: indices,
            eligibility,
        });
    }
    let decision = match device.ownership {
        Ownership::Resident(backend) => Decision::ReconcileResident(backend),
        Ownership::Unknown | Ownership::Foreign => Decision::BlockedOwnership,
        Ownership::Clear => candidates
            .iter()
            // Auto admits one kernel candidate. Eligibility of KPM/Zygisk is
            // still reported for explicit choices, never an implicit fallback.
            .find(|c| {
                matches!(c.backend, NativeBackend::Builtin | NativeBackend::Kmod)
                    && c.eligibility.is_ok()
            })
            .map(|c| Decision::Select(c.backend))
            .unwrap_or(Decision::NoEligibleCandidate),
    };
    Ok(Assessment {
        kmi,
        candidates,
        decision,
    })
}

fn runtime_ready(runtime: Runtime) -> Result<(), Rejection> {
    match runtime {
        Runtime::Ready => Ok(()),
        Runtime::Absent => Err(Rejection::RuntimeAbsent),
        Runtime::AuthenticationRequired => Err(Rejection::AuthenticationRequired),
        Runtime::Unsupported => Err(Rejection::UnsupportedRuntime),
        Runtime::Unknown => Err(Rejection::RuntimeUnknown),
    }
}

fn available(
    manifest: &Manifest,
    device: &Capabilities,
    kmi: &super::kmi::KmiResolution,
    backend: NativeBackend,
) -> (Vec<usize>, Result<(), Rejection>) {
    let indices: Vec<_> = manifest
        .payloads
        .iter()
        .enumerate()
        .filter_map(|(index, payload)| {
            let matches = match (&payload.implementation, backend) {
                (PayloadKind::Kmod { .. }, NativeBackend::Kmod) => {
                    kmi.candidates.iter().any(|c| c.payload_index == index)
                }
                (PayloadKind::Kpm, NativeBackend::Kpm) => true,
                (PayloadKind::Zygisk { abi }, NativeBackend::Zygisk) => {
                    *abi == device.abi
                        || (device.abi == Abi::Arm64
                            && device.needs_arm32_zygisk
                            && *abi == Abi::Arm32)
                }
                _ => false,
            };
            matches.then_some(index)
        })
        .collect();
    let result = match backend {
        NativeBackend::Builtin => Ok(()),
        NativeBackend::Kmod | NativeBackend::Kpm if device.abi != Abi::Arm64 => {
            Err(Rejection::WrongArchitecture)
        }
        NativeBackend::Kmod if kmi.series.is_none() => Err(Rejection::UnknownKmi),
        NativeBackend::Kmod if indices.is_empty() => Err(Rejection::NoMatchingKmi),
        NativeBackend::Kmod
            if kmi.conflicting_hints
                || (kmi.release_hint.is_some()
                    && !kmi.candidates.iter().any(|c| c.matches_release_hint)) =>
        {
            Err(Rejection::ConflictingKmiEvidence)
        }
        NativeBackend::Kmod if indices.len() != 1 => Err(Rejection::AmbiguousKmi),
        NativeBackend::Zygisk
            if indices.len()
                != if device.abi == Abi::Arm64 && device.needs_arm32_zygisk {
                    2
                } else {
                    1
                } =>
        {
            Err(Rejection::MissingPayload)
        }
        _ if indices.is_empty() => Err(Rejection::MissingPayload),
        _ => Ok(()),
    };
    (indices, result)
}
