//! Scoped framework fixture protocol. Source objects are deliberately dirty in
//! both phases; only the production hiding path may change protected observations.
use crate::{
    ProbeError, Sample,
    framework_legacy::{LegacyObservation, LegacySourceEvidence},
};
use serde::{Deserialize, Serialize};
pub const FRAMEWORK_PROTOCOL: u32 = 2;
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
    pub legacy: LegacySourceEvidence,
}
impl FrameworkAcknowledgement {
    pub fn matches(&self, envelope: &FrameworkEnvelope) -> bool {
        envelope.valid()
            && self.legacy.bounded()
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
    pub legacy: LegacyObservation,
}
impl FrameworkObservation {
    pub fn bounded(&self) -> bool {
        self.legacy.bounded()
            && self.networks.value().is_none_or(|v| {
                v.len() <= 256
                    && v.iter().all(|&h| h != 0)
                    && v.iter().collect::<std::collections::BTreeSet<_>>().len() == v.len()
            })
            && self.vpn_interface.value().is_none_or(|n| {
                n.as_ref()
                    .is_none_or(|n| n.len() <= 64 && !n.chars().any(char::is_control))
            })
            && [
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
            legacy: LegacyObservation::unavailable(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkCheck {
    Networks,
    Capabilities,
    LinkProperties,
    LegacyTypeVpn,
    LegacyEnumeration,
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
    leaked.extend(crate::framework_legacy::observed_leaks(raw, protected));
    leaked
}
/// Synchronous handle and legacy type/enumeration scenarios cannot discharge
/// default/request/callback or PendingIntent requirements. Leaks survive cleanup failure.
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
    let legacy = crate::framework_legacy::evaluate_legacy(phases, cleanup);
    result.proven.extend(legacy.proven);
    result.leaked.extend(legacy.leaked);
    result.controls.extend(legacy.controls);
    result.leaked.sort();
    result.leaked.dedup();
    result.controls.sort();
    result.controls.dedup();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn envelope() -> FrameworkEnvelope {
        FrameworkEnvelope {
            version: FRAMEWORK_PROTOCOL,
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
                    version: FRAMEWORK_PROTOCOL,
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
                    legacy: LegacySourceEvidence::default(),
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
                    legacy: LegacyObservation::unavailable(),
                },
            }
        })
        .collect()
    }
    fn legacy_phases() -> Vec<FrameworkPhaseEvidence> {
        use crate::framework_legacy::{LegacyNetworkInfo, LegacyOrdinary};
        let mut phases = phases();
        for (index, phase) in phases.iter_mut().enumerate() {
            let active = matches!(index, 1 | 2);
            let raw = index == 1;
            let info = LegacyNetworkInfo {
                network_type: 17,
                state: if raw { "CONNECTED" } else { "DISCONNECTED" }.into(),
                detailed_state: if raw { "CONNECTED" } else { "DISCONNECTED" }.into(),
                fixture: raw,
            };
            let ordinary = LegacyOrdinary {
                count: u32::from(active),
                sha256: format!("{index:064x}"),
                fixture_present: active,
            };
            phase.acknowledgement.legacy = LegacySourceEvidence {
                type_calls: 1,
                enumeration_calls: 1,
                type_started_boot_ns: 1000,
                enumeration_started_boot_ns: 2000,
                type_source: active,
                enumeration_source: active,
                type_normalized: index == 2,
                enumeration_normalized: index == 2,
                enumeration_count: active.then_some(2),
                ordinary: active.then_some(ordinary.clone()),
            };
            phase.observation.legacy = LegacyObservation {
                type_window: Some(crate::framework_legacy::LegacyCallWindow {
                    started_boot_ns: 900,
                    finished_boot_ns: 1100,
                }),
                enumeration_window: Some(crate::framework_legacy::LegacyCallWindow {
                    started_boot_ns: 1900,
                    finished_boot_ns: 2100,
                }),
                type_info: Sample::Value(active.then_some(info.clone())),
                enumeration: Sample::Value(if active {
                    vec![
                        info,
                        LegacyNetworkInfo {
                            network_type: 1,
                            state: "CONNECTED".into(),
                            detailed_state: "CONNECTED".into(),
                            fixture: false,
                        },
                    ]
                } else {
                    vec![]
                }),
                ordinary: Sample::Value(ordinary),
            };
        }
        phases
    }
    #[test]
    fn legacy_apis_require_both_production_paths_and_per_invocation_preservation() {
        use FrameworkCheck as V;
        let phases = legacy_phases();
        let result = evaluate_framework(&phases, true);
        assert!(result.controls.is_empty());
        assert_eq!(
            result.proven,
            vec![
                V::Networks,
                V::Capabilities,
                V::LinkProperties,
                V::LegacyTypeVpn,
                V::LegacyEnumeration
            ]
        );
        assert!(crate::assessment::framework_checks(&[V::LegacyTypeVpn], true).is_empty());
        assert!(crate::assessment::framework_checks(&[V::LegacyEnumeration], true).is_empty());
        assert_eq!(
            crate::assessment::framework_checks(&result.proven, true).len(),
            4
        );
        for edit in 0..8 {
            let mut bad = phases.clone();
            match edit {
                0 => bad[1].acknowledgement.legacy.type_source = false,
                1 => bad[2].acknowledgement.legacy.type_normalized = false,
                2 => bad[2].observation.legacy.type_info = Sample::Value(None),
                3 => {
                    if let Sample::Value(Some(info)) = &mut bad[2].observation.legacy.type_info {
                        info.network_type = 1;
                    }
                }
                4 => bad[2].acknowledgement.legacy.enumeration_source = false,
                5 => bad[2].acknowledgement.legacy.enumeration_normalized = false,
                6 => bad[2].acknowledgement.legacy.enumeration_count = Some(3),
                _ => {
                    if let Sample::Value(ordinary) = &mut bad[2].observation.legacy.ordinary {
                        ordinary.sha256 = "f".repeat(64);
                    }
                }
            }
            let result = evaluate_framework(&bad, true);
            assert!(!result.controls.is_empty(), "edit={edit}");
            assert!(
                !crate::assessment::framework_checks(&result.proven, true)
                    .contains(&crate::RequiredCheck::FrameworkLegacyInfo)
            );
            assert!(result.proven.contains(&if edit < 4 {
                V::LegacyEnumeration
            } else {
                V::LegacyTypeVpn
            }));
        }
    }
    #[test]
    fn legacy_negative_controls_are_marker_scoped_and_leaks_survive_lost_cleanup() {
        use FrameworkCheck as V;
        let mut phases = legacy_phases();
        // A real VPN at the negative baseline is allowed; fixture markers are not.
        let mut real = phases[1].observation.legacy.type_info.clone();
        if let Sample::Value(Some(info)) = &mut real {
            info.fixture = false;
        }
        phases[0].observation.legacy.type_info = real;
        assert!(
            evaluate_framework(&phases, true)
                .proven
                .contains(&V::LegacyTypeVpn)
        );
        phases[2].observation.legacy.type_info = phases[1].observation.legacy.type_info.clone();
        phases[2].observation.legacy.enumeration = phases[1].observation.legacy.enumeration.clone();
        for complete in [true, false] {
            let result = evaluate_framework(if complete { &phases } else { &phases[..3] }, false);
            assert!(result.proven.is_empty());
            assert!(result.leaked.contains(&V::LegacyTypeVpn));
            assert!(result.leaked.contains(&V::LegacyEnumeration));
        }
        phases[2].acknowledgement.worker_pid += 1;
        assert!(evaluate_framework(&phases, true).leaked.is_empty());
    }
    #[test]
    fn inactive_legacy_state_with_retained_vpn_marker_is_still_a_leak() {
        let mut evidence = legacy_phases();
        if let Sample::Value(Some(info)) = &mut evidence[2].observation.legacy.type_info {
            info.fixture = true;
        }
        if let Sample::Value(infos) = &mut evidence[2].observation.legacy.enumeration {
            infos[0].fixture = true;
        }
        let result = evaluate_framework(&evidence, true);
        assert!(result.leaked.contains(&FrameworkCheck::LegacyTypeVpn));
        assert!(result.leaked.contains(&FrameworkCheck::LegacyEnumeration));
        assert!(!result.proven.contains(&FrameworkCheck::LegacyTypeVpn));
        assert!(!result.proven.contains(&FrameworkCheck::LegacyEnumeration));
    }
    #[test]
    fn repeated_or_unobserved_legacy_binder_calls_cannot_borrow_older_normalization() {
        for count in [0, 2] {
            let mut phases = legacy_phases();
            phases[2].acknowledgement.legacy.type_calls = count;
            phases[2].acknowledgement.legacy.enumeration_calls = count;
            let result = evaluate_framework(&phases, true);
            assert!(!result.proven.contains(&FrameworkCheck::LegacyTypeVpn));
            assert!(!result.proven.contains(&FrameworkCheck::LegacyEnumeration));
            assert!(
                result
                    .controls
                    .contains(&FrameworkControlFailure::MissingPositiveControl)
            );
        }
    }
    #[test]
    fn old_ack_and_malformed_call_windows_cannot_prove_a_new_worker_query() {
        for edit in 0..5 {
            let mut phases = legacy_phases();
            match edit {
                0 => phases[2].acknowledgement.legacy.type_started_boot_ns = 899,
                1 => phases[2].acknowledgement.legacy.type_started_boot_ns = 1101,
                2 => phases[2].observation.legacy.type_window = None,
                3 => {
                    phases[2]
                        .observation
                        .legacy
                        .type_window
                        .as_mut()
                        .unwrap()
                        .finished_boot_ns = 899
                }
                _ => {
                    phases[2]
                        .observation
                        .legacy
                        .type_window
                        .as_mut()
                        .unwrap()
                        .finished_boot_ns = u64::MAX
                }
            }
            let result = evaluate_framework(&phases, true);
            assert!(
                !result.proven.contains(&FrameworkCheck::LegacyTypeVpn),
                "edit={edit}"
            );
        }
    }
    #[test]
    fn negative_legacy_windows_cannot_borrow_an_ack_from_another_invocation() {
        for phase in [0, 3] {
            for api in [0, 1] {
                for missing in [true, false] {
                    let mut phases = legacy_phases();
                    let value = &mut phases[phase];
                    match (api, missing) {
                        (0, true) => value.observation.legacy.type_window = None,
                        (0, false) => value.acknowledgement.legacy.type_started_boot_ns = 899,
                        (1, true) => value.observation.legacy.enumeration_window = None,
                        _ => value.acknowledgement.legacy.enumeration_started_boot_ns = 1899,
                    }
                    let result = evaluate_framework(&phases, true);
                    let (failed, other) = if api == 0 {
                        (
                            FrameworkCheck::LegacyTypeVpn,
                            FrameworkCheck::LegacyEnumeration,
                        )
                    } else {
                        (
                            FrameworkCheck::LegacyEnumeration,
                            FrameworkCheck::LegacyTypeVpn,
                        )
                    };
                    assert!(!result.proven.contains(&failed));
                    assert!(result.proven.contains(&other));
                    assert!(
                        result
                            .controls
                            .contains(&FrameworkControlFailure::MissingNegativeControl)
                    );
                }
            }
        }
    }
    #[test]
    fn timed_positive_legacy_leaks_survive_an_ambiguous_duplicate_call_count() {
        let mut phases = legacy_phases();
        phases[2].acknowledgement.legacy.type_calls = 2;
        phases[2].acknowledgement.legacy.enumeration_calls = 2;
        phases[2].observation.legacy.type_info = phases[1].observation.legacy.type_info.clone();
        phases[2].observation.legacy.enumeration = phases[1].observation.legacy.enumeration.clone();
        let result = evaluate_framework(&phases, true);
        assert!(result.leaked.contains(&FrameworkCheck::LegacyTypeVpn));
        assert!(result.leaked.contains(&FrameworkCheck::LegacyEnumeration));
        assert!(!result.proven.contains(&FrameworkCheck::LegacyTypeVpn));
        assert!(!result.proven.contains(&FrameworkCheck::LegacyEnumeration));
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
        assert_eq!(
            result.controls,
            vec![FrameworkControlFailure::MissingNegativeControl]
        );
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
    pub vpn_marker: String,
    pub ordinary_marker: String,
}
