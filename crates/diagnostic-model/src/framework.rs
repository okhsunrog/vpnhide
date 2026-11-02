//! Scoped framework fixture protocol. Source objects are deliberately dirty in
//! both phases; only the production hiding path may change protected observations.
use crate::{ProbeError, Sample};
use serde::{Deserialize, Serialize};
pub const FRAMEWORK_PROTOCOL: u32 = 1;
pub const FRAMEWORK_LEASE_MS: u64 = 25_000;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkPhase {
    Prepared,
    Raw,
    Protected,
    Revoked,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameworkFixtureSpec {
    pub vpn_net_id: u32,
    pub ordinary_net_id: u32,
    pub interface_name: String,
    pub mtu: u32,
    pub ordinary_interface_name: String,
    pub ordinary_mtu: u32,
}
impl FrameworkFixtureSpec {
    pub fn valid(&self) -> bool {
        // These are invocation-local synthetic handles, never global netd IDs.
        self.vpn_net_id > 65_535
            && self.vpn_net_id <= i32::MAX as u32
            && self.ordinary_net_id > 65_535
            && self.ordinary_net_id <= i32::MAX as u32
            && self.vpn_net_id != self.ordinary_net_id
            && self.interface_name.len() == 15
            && self.interface_name.starts_with("tun")
            && self.interface_name[3..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            && (1..=65_535).contains(&self.mtu)
            && self.ordinary_interface_name.starts_with("ipsec")
            && self.ordinary_interface_name.len() <= 15
            && self
                .ordinary_interface_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric())
            && (1..=65_535).contains(&self.ordinary_mtu)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameworkEnvelope {
    pub version: u32,
    pub session: u64,
    pub core_epoch: u64,
    pub boot_id: String,
    pub nonce: String,
    pub expected_hook_sha256: String,
    pub worker_uid: u32,
    pub worker_pid: u32,
    pub worker_start_ticks: u64,
    pub expires_boot_ms: u64,
    pub generation: u32,
    pub phase: FrameworkPhase,
    pub fixture: FrameworkFixtureSpec,
}
impl FrameworkEnvelope {
    pub fn valid(&self) -> bool {
        self.version == FRAMEWORK_PROTOCOL
            && self.session > 0
            && self.session <= i64::MAX as u64
            && self.core_epoch > 0
            && self.core_epoch <= i64::MAX as u64
            && self.boot_id.len() == 36
            && self.boot_id.bytes().enumerate().all(|(i, b)| {
                if matches!(i, 8 | 13 | 18 | 23) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
                }
            })
            && self.nonce.len() == 64
            && self
                .nonce
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            && self.expected_hook_sha256.len() == 64
            && self
                .expected_hook_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            && self.worker_uid >= 10_000
            && self.worker_uid <= i32::MAX as u32
            && self.worker_pid > 1
            && self.worker_pid <= i32::MAX as u32
            && self.worker_start_ticks > 0
            && self.expires_boot_ms > 0
            && self.expires_boot_ms <= i64::MAX as u64
            && self.generation
                == match self.phase {
                    FrameworkPhase::Prepared => 0,
                    FrameworkPhase::Raw => 1,
                    FrameworkPhase::Protected => 2,
                    FrameworkPhase::Revoked => 3,
                }
            && self.fixture.valid()
    }
    pub fn permits(&self, boot: &str, uid: u32, pid: u32, start: u64, now: u64) -> bool {
        self.valid()
            && self.boot_id == boot
            && self.worker_uid == uid
            && self.worker_pid == pid
            && self.worker_start_ticks == start
            && now < self.expires_boot_ms
            && self.expires_boot_ms - now <= FRAMEWORK_LEASE_MS
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameworkSourceIdentity {
    /// Public Network.getNetworkHandle() from actual constructed Android objects.
    /// Core treats these as opaque and never guesses Android's handle encoding.
    pub vpn_handle: u64,
    pub ordinary_handle: u64,
    pub profile_sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameworkAcknowledgement {
    pub version: u32,
    pub session: u64,
    pub core_epoch: u64,
    pub nonce: String,
    pub generation: u32,
    pub phase: FrameworkPhase,
    pub worker_uid: u32,
    pub worker_pid: u32,
    pub worker_start_ticks: u64,
    pub sources: FrameworkSourceIdentity,
    pub fixture: FrameworkFixtureSpec,
    pub system_server_pid: u32,
    pub system_server_start_ticks: u64,
    pub hook_sha256: String,
    pub networks_source: bool,
    pub capabilities_source: bool,
    pub properties_source: bool,
    pub networks_filtered: bool,
    pub capabilities_filtered: bool,
    pub properties_filtered: bool,
}
impl FrameworkAcknowledgement {
    pub fn matches(&self, envelope: &FrameworkEnvelope) -> bool {
        envelope.valid()
            && self.version == FRAMEWORK_PROTOCOL
            && self.session == envelope.session
            && self.core_epoch == envelope.core_epoch
            && self.nonce == envelope.nonce
            && self.generation == envelope.generation
            && self.phase == envelope.phase
            && self.worker_uid == envelope.worker_uid
            && self.worker_pid == envelope.worker_pid
            && self.worker_start_ticks == envelope.worker_start_ticks
            && self.fixture == envelope.fixture
            && self.system_server_pid > 1
            && self.system_server_start_ticks > 0
            && self.hook_sha256 == envelope.expected_hook_sha256
            && self.sources.vpn_handle != 0
            && self.sources.ordinary_handle != 0
            && self.sources.vpn_handle != self.sources.ordinary_handle
            && self.sources.profile_sha256.len() == 64
            && self
                .sources
                .profile_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameworkObservation {
    pub networks: Sample<Vec<u64>>,
    pub vpn_capabilities_present: Sample<bool>,
    pub vpn_transport: Sample<bool>,
    pub vpn_properties_present: Sample<bool>,
    pub vpn_interface: Sample<Option<String>>,
    pub ordinary_capabilities_sha256: Sample<String>,
    pub ordinary_properties_sha256: Sample<String>,
}
impl FrameworkObservation {
    pub fn bounded(&self) -> bool {
        self.networks.value().is_none_or(|v| {
            v.len() <= 256
                && v.iter().all(|&h| h != 0)
                && v.iter().collect::<std::collections::BTreeSet<_>>().len() == v.len()
        }) && self.vpn_interface.value().is_none_or(|n| {
            n.as_ref()
                .is_none_or(|n| n.len() <= 64 && !n.chars().any(char::is_control))
        }) && [
            &self.ordinary_capabilities_sha256,
            &self.ordinary_properties_sha256,
        ]
        .into_iter()
        .all(|v| {
            v.value()
                .is_none_or(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        })
    }
    pub fn unavailable() -> Self {
        Self {
            networks: Sample::Error(ProbeError::Unsupported),
            vpn_capabilities_present: Sample::Error(ProbeError::Unsupported),
            vpn_transport: Sample::Error(ProbeError::Unsupported),
            vpn_properties_present: Sample::Error(ProbeError::Unsupported),
            vpn_interface: Sample::Error(ProbeError::Unsupported),
            ordinary_capabilities_sha256: Sample::Error(ProbeError::Unsupported),
            ordinary_properties_sha256: Sample::Error(ProbeError::Unsupported),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkCheck {
    Networks,
    Capabilities,
    LinkProperties,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkControlFailure {
    InvalidEnvelope,
    IdentityChanged,
    MissingNegativeControl,
    MissingPositiveControl,
    SourceChanged,
    ProductionPathNotObserved,
    OrdinaryObjectsChanged,
    ProbeUnavailable,
    CleanupUnconfirmed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameworkPhaseEvidence {
    pub envelope: FrameworkEnvelope,
    pub acknowledgement: FrameworkAcknowledgement,
    pub observation: FrameworkObservation,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameworkAttribution {
    pub session: u64,
    pub core_epoch: u64,
    pub boot_id: String,
    pub worker_uid: u32,
    pub worker_pid: u32,
    pub worker_start_ticks: u64,
    pub system_server_pid: u32,
    pub system_server_start_ticks: u64,
    pub hook_source_sha256: String,
    pub source_profile_sha256: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedFrameworkResult {
    pub attribution: Option<FrameworkAttribution>,
    pub proven: Vec<FrameworkCheck>,
    pub leaked: Vec<FrameworkCheck>,
    pub controls: Vec<FrameworkControlFailure>,
}
impl FrameworkEnvelope {
    pub fn for_phase(&self, phase: FrameworkPhase) -> Self {
        let mut next = self.clone();
        next.phase = phase;
        next.generation = match phase {
            FrameworkPhase::Prepared => 0,
            FrameworkPhase::Raw => 1,
            FrameworkPhase::Protected => 2,
            FrameworkPhase::Revoked => 3,
        };
        next
    }
}
fn absent(observed: &FrameworkObservation, source: &FrameworkSourceIdentity) -> bool {
    observed
        .networks
        .value()
        .is_some_and(|v| !v.contains(&source.vpn_handle) && !v.contains(&source.ordinary_handle))
        && observed.vpn_capabilities_present.value() == Some(&false)
        && observed.vpn_properties_present.value() == Some(&false)
}
fn observed_leaks(phases: &[FrameworkPhaseEvidence]) -> Vec<FrameworkCheck> {
    let Some([prepared, raw, protected]) = phases.get(..3) else {
        return vec![];
    };
    let identity = &prepared.acknowledgement;
    for (e, phase) in [prepared, raw, protected].into_iter().zip([
        FrameworkPhase::Prepared,
        FrameworkPhase::Raw,
        FrameworkPhase::Protected,
    ]) {
        if e.envelope != prepared.envelope.for_phase(phase)
            || !e.acknowledgement.matches(&e.envelope)
            || !e.observation.bounded()
            || e.acknowledgement.sources != identity.sources
            || e.acknowledgement.system_server_pid != identity.system_server_pid
            || e.acknowledgement.system_server_start_ticks != identity.system_server_start_ticks
        {
            return vec![];
        }
    }
    let source = &identity.sources;
    let mut leaked = vec![];
    if raw.acknowledgement.networks_source
        && protected.acknowledgement.networks_source
        && raw
            .observation
            .networks
            .value()
            .is_some_and(|v| v.contains(&source.vpn_handle))
        && protected
            .observation
            .networks
            .value()
            .is_some_and(|v| v.contains(&source.vpn_handle))
    {
        leaked.push(FrameworkCheck::Networks);
    }
    if raw.acknowledgement.capabilities_source
        && protected.acknowledgement.capabilities_source
        && raw.observation.vpn_capabilities_present.value() == Some(&true)
        && raw.observation.vpn_transport.value() == Some(&true)
        && protected.observation.vpn_capabilities_present.value() == Some(&true)
    {
        leaked.push(FrameworkCheck::Capabilities);
    }
    if raw.acknowledgement.properties_source
        && protected.acknowledgement.properties_source
        && raw.observation.vpn_properties_present.value() == Some(&true)
        && raw.observation.vpn_interface.value()
            == Some(&Some(raw.envelope.fixture.interface_name.clone()))
        && protected.observation.vpn_properties_present.value() == Some(&true)
    {
        leaked.push(FrameworkCheck::LinkProperties);
    }
    leaked
}
/// The three synchronous scenarios cannot discharge default/request/callback,
/// legacy or PendingIntent requirements. Confirmed leaks survive cleanup failure.
pub fn evaluate_framework(
    phases: &[FrameworkPhaseEvidence],
    cleanup: bool,
) -> ScopedFrameworkResult {
    use FrameworkCheck as V;
    use FrameworkControlFailure as F;
    let mut result = ScopedFrameworkResult {
        leaked: observed_leaks(phases),
        attribution: phases
            .first()
            .filter(|e| {
                e.envelope.phase == FrameworkPhase::Prepared
                    && e.acknowledgement.matches(&e.envelope)
            })
            .map(|e| FrameworkAttribution {
                session: e.envelope.session,
                core_epoch: e.envelope.core_epoch,
                boot_id: e.envelope.boot_id.clone(),
                worker_uid: e.envelope.worker_uid,
                worker_pid: e.envelope.worker_pid,
                worker_start_ticks: e.envelope.worker_start_ticks,
                system_server_pid: e.acknowledgement.system_server_pid,
                system_server_start_ticks: e.acknowledgement.system_server_start_ticks,
                hook_source_sha256: e.acknowledgement.hook_sha256.clone(),
                source_profile_sha256: e.acknowledgement.sources.profile_sha256.clone(),
            }),
        ..Default::default()
    };
    if !cleanup {
        result.controls.push(F::CleanupUnconfirmed);
    }
    let [prepared, raw, protected, revoked] = phases else {
        result.controls.push(F::MissingNegativeControl);
        return result;
    };
    for (e, phase) in phases.iter().zip([
        FrameworkPhase::Prepared,
        FrameworkPhase::Raw,
        FrameworkPhase::Protected,
        FrameworkPhase::Revoked,
    ]) {
        if e.envelope != prepared.envelope.for_phase(phase)
            || !e.acknowledgement.matches(&e.envelope)
            || !e.observation.bounded()
        {
            result.controls.push(F::InvalidEnvelope);
            return result;
        }
    }
    let original = &prepared.acknowledgement;
    if phases.iter().any(|e| {
        e.acknowledgement.system_server_pid != original.system_server_pid
            || e.acknowledgement.system_server_start_ticks != original.system_server_start_ticks
            || e.acknowledgement.hook_sha256 != original.hook_sha256
    }) {
        result.controls.push(F::IdentityChanged);
        return result;
    }
    if phases
        .iter()
        .any(|e| e.acknowledgement.sources != original.sources)
    {
        result.controls.push(F::SourceChanged);
        return result;
    }
    let source = &original.sources;
    let quiet = |a: &FrameworkAcknowledgement| {
        !a.networks_source
            && !a.capabilities_source
            && !a.properties_source
            && !a.networks_filtered
            && !a.capabilities_filtered
            && !a.properties_filtered
    };
    if !absent(&prepared.observation, source)
        || !absent(&revoked.observation, source)
        || !quiet(&prepared.acknowledgement)
        || !quiet(&revoked.acknowledgement)
    {
        result.controls.push(F::MissingNegativeControl);
    }
    let ordinary = raw
        .observation
        .ordinary_capabilities_sha256
        .value()
        .is_some()
        && raw.observation.ordinary_properties_sha256.value().is_some()
        && raw.observation.ordinary_capabilities_sha256
            == protected.observation.ordinary_capabilities_sha256
        && raw.observation.ordinary_properties_sha256
            == protected.observation.ordinary_properties_sha256
        && protected
            .observation
            .networks
            .value()
            .is_some_and(|v| v.contains(&source.ordinary_handle));
    if !ordinary {
        result.controls.push(F::OrdinaryObjectsChanged);
    }
    let positive = [
        raw.acknowledgement.networks_source
            && raw.observation.networks.value().is_some_and(|v| {
                v.contains(&source.vpn_handle) && v.contains(&source.ordinary_handle)
            }),
        raw.acknowledgement.capabilities_source
            && raw.observation.vpn_capabilities_present.value() == Some(&true)
            && raw.observation.vpn_transport.value() == Some(&true),
        raw.acknowledgement.properties_source
            && raw.observation.vpn_properties_present.value() == Some(&true)
            && raw.observation.vpn_interface.value()
                == Some(&Some(raw.envelope.fixture.interface_name.clone())),
    ];
    let injected = [
        protected.acknowledgement.networks_source,
        protected.acknowledgement.capabilities_source,
        protected.acknowledgement.properties_source,
    ];
    let production = [
        protected.acknowledgement.networks_filtered,
        protected.acknowledgement.capabilities_filtered,
        protected.acknowledgement.properties_filtered,
    ];
    let clean = [
        protected
            .observation
            .networks
            .value()
            .map(|v| !v.contains(&source.vpn_handle)),
        protected
            .observation
            .vpn_capabilities_present
            .value()
            .map(|v| !*v),
        protected
            .observation
            .vpn_properties_present
            .value()
            .map(|v| !*v),
    ];
    for (index, vector) in [V::Networks, V::Capabilities, V::LinkProperties]
        .into_iter()
        .enumerate()
    {
        if !positive[index] || !injected[index] {
            result.controls.push(F::MissingPositiveControl);
            continue;
        }
        match clean[index] {
            Some(false) => result.leaked.push(vector),
            Some(true) if production[index] => result.proven.push(vector),
            Some(true) => result.controls.push(F::ProductionPathNotObserved),
            None => result.controls.push(F::ProbeUnavailable),
        }
    }
    result.leaked.sort();
    result.leaked.dedup();
    result.controls.sort();
    result.controls.dedup();
    if !result.controls.is_empty() {
        result.proven.clear();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn envelope() -> FrameworkEnvelope {
        FrameworkEnvelope {
            version: 1,
            session: 9,
            core_epoch: 7,
            boot_id: "12345678-1234-1234-1234-123456789abc".into(),
            nonce: "a".repeat(64),
            expected_hook_sha256: "c".repeat(64),
            worker_uid: 10123,
            worker_pid: 444,
            worker_start_ticks: 1234,
            expires_boot_ms: 25_000,
            generation: 0,
            phase: FrameworkPhase::Prepared,
            fixture: FrameworkFixtureSpec {
                vpn_net_id: 0x6000_0001,
                ordinary_net_id: 0x6000_0002,
                interface_name: "tun000000000009".into(),
                mtu: 1543,
                ordinary_interface_name: "ipsec0000000009".into(),
                ordinary_mtu: 1519,
            },
        }
    }
    fn phases() -> Vec<FrameworkPhaseEvidence> {
        let base = envelope();
        let source = FrameworkSourceIdentity {
            vpn_handle: 101,
            ordinary_handle: 102,
            profile_sha256: "d".repeat(64),
        };
        [
            FrameworkPhase::Prepared,
            FrameworkPhase::Raw,
            FrameworkPhase::Protected,
            FrameworkPhase::Revoked,
        ]
        .into_iter()
        .map(|phase| {
            let active = matches!(phase, FrameworkPhase::Raw | FrameworkPhase::Protected);
            let raw = phase == FrameworkPhase::Raw;
            let protected = phase == FrameworkPhase::Protected;
            FrameworkPhaseEvidence {
                envelope: base.for_phase(phase),
                acknowledgement: FrameworkAcknowledgement {
                    version: 1,
                    session: 9,
                    core_epoch: 7,
                    nonce: base.nonce.clone(),
                    generation: base.for_phase(phase).generation,
                    phase,
                    worker_uid: base.worker_uid,
                    worker_pid: base.worker_pid,
                    worker_start_ticks: base.worker_start_ticks,
                    sources: source.clone(),
                    fixture: base.fixture.clone(),
                    system_server_pid: 99,
                    system_server_start_ticks: 555,
                    hook_sha256: base.expected_hook_sha256.clone(),
                    networks_source: active,
                    capabilities_source: active,
                    properties_source: active,
                    networks_filtered: protected,
                    capabilities_filtered: protected,
                    properties_filtered: protected,
                },
                observation: FrameworkObservation {
                    networks: Sample::Value(if raw {
                        vec![11, 101, 102]
                    } else if protected {
                        vec![11, 102]
                    } else {
                        vec![11]
                    }),
                    vpn_capabilities_present: Sample::Value(raw),
                    vpn_transport: Sample::Value(raw),
                    vpn_properties_present: Sample::Value(raw),
                    vpn_interface: Sample::Value(raw.then(|| base.fixture.interface_name.clone())),
                    ordinary_capabilities_sha256: Sample::Value("1".repeat(64)),
                    ordinary_properties_sha256: Sample::Value("2".repeat(64)),
                },
            }
        })
        .collect()
    }
    #[test]
    fn authority_is_exact_process_boot_and_bounded_boottime() {
        let env = envelope();
        assert!(env.valid());
        assert!(env.permits(&env.boot_id, 10123, 444, 1234, 1));
        for (boot, uid, pid, start, now) in [
            (env.boot_id.as_str(), 10123, 445, 1234, 1),
            (env.boot_id.as_str(), 10123, 444, 1235, 1),
            (env.boot_id.as_str(), 10124, 444, 1234, 1),
            (env.boot_id.as_str(), 10123, 444, 1234, 25_000),
            ("other", 10123, 444, 1234, 1),
        ] {
            assert!(!env.permits(boot, uid, pid, start, now));
        }
        let mut long = env.clone();
        long.expires_boot_ms = 50_000;
        assert!(!long.permits(&long.boot_id, 10123, 444, 1234, 1));
    }
    #[test]
    fn three_real_path_proofs_do_not_satisfy_the_remaining_framework_contract() {
        let result = evaluate_framework(&phases(), true);
        assert_eq!(
            result.proven,
            vec![
                FrameworkCheck::Networks,
                FrameworkCheck::Capabilities,
                FrameworkCheck::LinkProperties
            ]
        );
        assert!(result.leaked.is_empty());
        assert!(result.controls.is_empty());
        let plan = crate::DiagnosticPlan::supported(false, false, false);
        for missing in [
            crate::RequiredCheck::FrameworkCallbacks,
            crate::RequiredCheck::FrameworkPendingIntent,
            crate::RequiredCheck::FrameworkLegacyInfo,
        ] {
            assert!(plan.required.contains(&missing));
        }
    }
    #[test]
    fn missing_raw_and_clean_worker_answers_are_not_proof_without_actual_production_branch() {
        let mut evidence = phases();
        evidence[1].observation.networks = Sample::Value(vec![]);
        let result = evaluate_framework(&evidence, true);
        assert!(result.proven.is_empty());
        assert!(result.leaked.is_empty());
        assert!(
            result
                .controls
                .contains(&FrameworkControlFailure::MissingPositiveControl)
        );
        let mut evidence = phases();
        evidence[2].acknowledgement.capabilities_filtered = false;
        let result = evaluate_framework(&evidence, true);
        assert!(result.proven.is_empty());
        assert!(
            result
                .controls
                .contains(&FrameworkControlFailure::ProductionPathNotObserved)
        );
    }
    #[test]
    fn ordinary_mutation_source_change_or_system_server_restart_withdraws_proof() {
        for change in 0..3 {
            let mut evidence = phases();
            match change {
                0 => {
                    evidence[2].observation.ordinary_properties_sha256 =
                        Sample::Value("3".repeat(64))
                }
                1 => evidence[2].acknowledgement.sources.profile_sha256 = "e".repeat(64),
                _ => evidence[2].acknowledgement.system_server_start_ticks += 1,
            }
            let result = evaluate_framework(&evidence, true);
            assert!(result.proven.is_empty());
            assert!(!result.controls.is_empty());
        }
    }
    #[test]
    fn confirmed_leaks_survive_cleanup_uncertainty() {
        let mut evidence = phases();
        evidence[2].observation.vpn_capabilities_present = Sample::Value(true);
        let result = evaluate_framework(&evidence, false);
        assert_eq!(result.leaked, vec![FrameworkCheck::Capabilities]);
        assert!(result.proven.is_empty());
        assert!(
            result
                .controls
                .contains(&FrameworkControlFailure::CleanupUnconfirmed)
        );
    }
    #[test]
    fn captured_protected_leaks_survive_worker_death_or_invalid_final_negative_control() {
        let mut evidence = phases();
        evidence[2].observation.vpn_capabilities_present = Sample::Value(true);
        let partial = evaluate_framework(&evidence[..3], true);
        assert_eq!(partial.leaked, vec![FrameworkCheck::Capabilities]);
        assert!(partial.proven.is_empty());
        evidence[3].acknowledgement.worker_pid += 1;
        let invalid_final = evaluate_framework(&evidence, false);
        assert_eq!(invalid_final.leaked, partial.leaked);
        assert!(invalid_final.proven.is_empty());
    }
    #[test]
    fn old_phase_or_other_worker_evidence_cannot_be_relabelled_as_protected() {
        let mut evidence = phases();
        evidence[2].acknowledgement.generation = 1;
        assert_eq!(
            evaluate_framework(&evidence, true).controls,
            vec![FrameworkControlFailure::InvalidEnvelope]
        );
        let mut evidence = phases();
        evidence[2].acknowledgement.worker_pid += 1;
        assert_eq!(
            evaluate_framework(&evidence, true).controls,
            vec![FrameworkControlFailure::InvalidEnvelope]
        );
    }
}

/// Explicit desired phase makes a lost-reply retry idempotent; expiry is never extended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkWorkerPhase {
    Prepared,
    Raw,
    Protected,
    Revoked,
    Finished,
}
impl FrameworkWorkerPhase {
    pub fn authority_phase(self) -> Option<FrameworkPhase> {
        match self {
            Self::Prepared => Some(FrameworkPhase::Prepared),
            Self::Raw => Some(FrameworkPhase::Raw),
            Self::Protected => Some(FrameworkPhase::Protected),
            Self::Revoked => Some(FrameworkPhase::Revoked),
            Self::Finished => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameworkWorkerRequest {
    pub session: u64,
    pub core_epoch: u64,
    pub hook_sha256: String,
    pub phase: FrameworkWorkerPhase,
    pub previous: Option<FrameworkObservation>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameworkWorkerTask {
    pub phase: FrameworkWorkerPhase,
    pub vpn_handle: u64,
    pub ordinary_handle: u64,
}
