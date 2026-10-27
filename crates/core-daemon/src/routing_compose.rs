//! Compose netlink candidates + framework classification into a RoutingObservation
//! with honest freshness / last-good semantics (core-2.0 §6/§8).

use std::collections::BTreeSet;

use vpnhide_checks::self_routed_for_interfaces;
use vpnhide_core_rpc::{
    FrameworkVpnEvidence, FrameworkVpnStatus, ObservationFreshness, RoutingObservation,
    UidRoutingObservation, UidVpnRouteState,
};

/// Env: optional UID for membership evaluation after classification.
pub const OBSERVE_UID_ENV: &str = "VPNHIDE_OBSERVE_UID";

pub fn observe_uid_from_env() -> Option<u32> {
    if let Ok(value) = std::env::var(OBSERVE_UID_ENV) {
        return value.trim().parse().ok();
    }
    use std::os::unix::fs::MetadataExt;
    // The same main-profile app directory gates IPC. Resolve it for each sample so
    // first install and reinstall never leave membership pinned to an obsolete UID.
    let dir = crate::app_socket::app_data_dir_from_env()?;
    std::fs::metadata(dir)
        .ok()
        .map(|metadata| metadata.uid())
        .filter(|uid| *uid >= 10_000)
}

/// Build the next routing observation.
///
/// - Framework Classified → Current (intersection of up candidates ∩ framework VPN set).
/// - Framework Unavailable/Failed/Never → Unknown; retain last-good; clear current ifaces.
/// - Netlink candidates alone never yield Current.
pub fn compose(
    previous: &RoutingObservation,
    generation: u64,
    netlink_up: &BTreeSet<String>,
    framework: FrameworkVpnEvidence,
    focus_uid: Option<u32>,
) -> RoutingObservation {
    let candidates: Vec<String> = netlink_up.iter().cloned().collect();
    let last_good_fallback = if previous.freshness == ObservationFreshness::Current {
        previous.vpn_interfaces.clone()
    } else {
        previous.last_good_vpn_interfaces.clone()
    };

    match framework.status {
        FrameworkVpnStatus::Classified => {
            let framework_set: BTreeSet<_> = framework.interfaces.iter().cloned().collect();
            let current: Vec<String> = netlink_up
                .iter()
                .filter(|name| framework_set.contains(*name))
                .cloned()
                .collect();
            // If framework reports VPN ifaces that are down, they stay out of current;
            // empty current with Classified means "VPN off or not up" — still Current.
            let uid = focus_uid.map(|uid| uid_membership(uid, &current, &framework));
            RoutingObservation {
                freshness: ObservationFreshness::Current,
                generation,
                last_good_vpn_interfaces: current.clone(),
                vpn_interfaces: current,
                netlink_candidates: candidates,
                framework,
                uid,
            }
        }
        FrameworkVpnStatus::Never
        | FrameworkVpnStatus::Unavailable
        | FrameworkVpnStatus::Failed => {
            let uid = focus_uid.map(|uid| UidRoutingObservation {
                uid,
                state: UidVpnRouteState::Unknown,
                detail: format!("framework_{:?}", framework.status).to_ascii_lowercase(),
                session: None,
                interfaces: Vec::new(),
            });
            RoutingObservation {
                freshness: ObservationFreshness::Unknown,
                generation,
                vpn_interfaces: Vec::new(),
                last_good_vpn_interfaces: last_good_fallback,
                netlink_candidates: candidates,
                framework,
                uid,
            }
        }
    }
}

fn uid_membership(
    uid: u32,
    current_vpn_ifaces: &[String],
    framework: &FrameworkVpnEvidence,
) -> UidRoutingObservation {
    if framework.interfaces.is_empty() {
        return UidRoutingObservation {
            uid,
            state: UidVpnRouteState::VpnOff,
            detail: "no_vpn".into(),
            session: None,
            interfaces: Vec::new(),
        };
    }
    // Membership uses the framework VPN set (authoritative), not only currently-up
    // intersection — matches vpnhide-core observe app-vpn-state.
    let (routed, detail) = self_routed_for_interfaces(uid, Some(&framework.interfaces));
    let _ = current_vpn_ifaces;
    match routed {
        Some(true) => UidRoutingObservation {
            uid,
            state: UidVpnRouteState::Routed,
            detail,
            session: Some(framework.detail.clone()),
            interfaces: framework.interfaces.clone(),
        },
        Some(false) => UidRoutingObservation {
            uid,
            state: UidVpnRouteState::Excluded,
            detail,
            session: Some(framework.detail.clone()),
            interfaces: framework.interfaces.clone(),
        },
        None => UidRoutingObservation {
            uid,
            state: UidVpnRouteState::Unknown,
            detail,
            session: Some(framework.detail.clone()),
            interfaces: framework.interfaces.clone(),
        },
    }
}

/// Mark Unknown while retaining last-good (overflow / pre-resync / observer failure).
pub fn mark_unknown(
    previous: &RoutingObservation,
    generation: u64,
    detail: &str,
) -> RoutingObservation {
    let last_good = if previous.freshness == ObservationFreshness::Current {
        previous.vpn_interfaces.clone()
    } else {
        previous.last_good_vpn_interfaces.clone()
    };
    RoutingObservation {
        freshness: ObservationFreshness::Unknown,
        generation,
        vpn_interfaces: Vec::new(),
        last_good_vpn_interfaces: last_good,
        netlink_candidates: previous.netlink_candidates.clone(),
        framework: FrameworkVpnEvidence {
            status: FrameworkVpnStatus::Unavailable,
            interfaces: previous.framework.interfaces.clone(),
            detail: detail.into(),
        },
        uid: previous.uid.as_ref().map(|u| UidRoutingObservation {
            uid: u.uid,
            state: UidVpnRouteState::Unknown,
            detail: detail.into(),
            session: u.session.clone(),
            interfaces: u.interfaces.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpnhide_core_rpc::FrameworkVpnEvidence;

    #[test]
    fn classified_intersection_is_current() {
        let prev = RoutingObservation::default();
        let mut up = BTreeSet::new();
        up.insert("tun0".into());
        up.insert("ipsec20".into());
        let fw = FrameworkVpnEvidence {
            status: FrameworkVpnStatus::Classified,
            interfaces: vec!["tun0".into()],
            detail: "test".into(),
        };
        let next = compose(&prev, 1, &up, fw, None);
        assert_eq!(next.freshness, ObservationFreshness::Current);
        assert_eq!(next.vpn_interfaces, vec!["tun0"]);
        assert_eq!(next.netlink_candidates, vec!["ipsec20", "tun0"]);
        assert_eq!(next.last_good_vpn_interfaces, vec!["tun0"]);
    }

    #[test]
    fn unavailable_framework_is_unknown_retains_last_good() {
        let prev = RoutingObservation {
            freshness: ObservationFreshness::Current,
            generation: 1,
            vpn_interfaces: vec!["tun0".into()],
            last_good_vpn_interfaces: vec!["tun0".into()],
            ..Default::default()
        };
        let up = BTreeSet::from(["tun0".into()]);
        let fw = FrameworkVpnEvidence {
            status: FrameworkVpnStatus::Unavailable,
            interfaces: Vec::new(),
            detail: "no_dumpsys".into(),
        };
        let next = compose(&prev, 2, &up, fw, None);
        assert_eq!(next.freshness, ObservationFreshness::Unknown);
        assert!(next.vpn_interfaces.is_empty());
        assert_eq!(next.last_good_vpn_interfaces, vec!["tun0"]);
    }

    #[test]
    fn netlink_only_never_becomes_current() {
        let prev = RoutingObservation::default();
        let up = BTreeSet::from(["tun0".into()]);
        let fw = FrameworkVpnEvidence {
            status: FrameworkVpnStatus::Never,
            ..Default::default()
        };
        let next = compose(&prev, 1, &up, fw, None);
        assert_eq!(next.freshness, ObservationFreshness::Unknown);
        assert!(next.vpn_interfaces.is_empty());
    }

    #[test]
    fn mark_unknown_keeps_last_good_without_claiming_current() {
        let prev = RoutingObservation {
            freshness: ObservationFreshness::Current,
            generation: 5,
            vpn_interfaces: vec!["wg0".into()],
            last_good_vpn_interfaces: vec!["wg0".into()],
            netlink_candidates: vec!["wg0".into()],
            ..Default::default()
        };
        let next = mark_unknown(&prev, 6, "enobufs");
        assert_eq!(next.freshness, ObservationFreshness::Unknown);
        assert!(next.vpn_interfaces.is_empty());
        assert_eq!(next.last_good_vpn_interfaces, vec!["wg0"]);
    }
}
