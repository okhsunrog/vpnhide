use serde::{Deserialize, Serialize};

use crate::{NativeFixtureSpec, NativeMeasurement, ProbeError, Sample};

/// The app's request was denied both with the filter bypassed and with it active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkRequestDenial {
    pub raw_errno: i32,
    pub protected_errno: i32,
}

fn send_denial(observation: &NativeMeasurement) -> Option<i32> {
    match observation.links {
        // Linux EPERM and EACCES. A receive-side error or a generic errno from
        // opening the socket is not evidence that the RTM_GETLINK send was denied.
        Sample::Error(ProbeError::GetlinkSendDenied(errno @ (1 | 13))) => Some(errno),
        _ => None,
    }
}

pub fn evaluate_link_request_denial(
    spec: &NativeFixtureSpec,
    samples: [&NativeMeasurement; 5],
    app_raw: Option<&NativeMeasurement>,
) -> Option<LinkRequestDenial> {
    let [negative, root_start, protected, root_end, cleanup] = samples;
    let app_raw = app_raw?;
    if !spec.valid()
        || !samples
            .into_iter()
            .chain([app_raw])
            .all(NativeMeasurement::bounded)
    {
        return None;
    }
    let control = |sample: &NativeMeasurement, present: bool| {
        sample.links.value().is_some_and(|links| {
            links
                .iter()
                .any(|link| link.matches(&spec.ordinary.interface))
                && if present {
                    links.iter().any(|link| link.matches(&spec.interface))
                } else {
                    links.iter().all(|link| {
                        link.name != spec.interface.name && link.index != Some(spec.interface.index)
                    })
                }
        })
    };
    if !control(negative, false)
        || !control(root_start, true)
        || !control(root_end, true)
        || !control(cleanup, false)
    {
        return None;
    }
    Some(LinkRequestDenial {
        raw_errno: send_denial(app_raw)?,
        protected_errno: send_denial(protected)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InterfaceId, ObservedInterface, OrdinaryControl};

    #[test]
    fn requires_both_attributed_sends_and_all_root_link_controls() {
        let spec = NativeFixtureSpec {
            session: 1,
            interface: InterfaceId {
                name: "tun000000000001".into(),
                index: 9,
            },
            flags: 16,
            mtu: 1543,
            ordinary: OrdinaryControl {
                interface: InterfaceId {
                    name: "lo".into(),
                    index: 1,
                },
                flags: 73,
                mtu: 65536,
            },
        };
        let link = |id: &InterfaceId| ObservedInterface {
            name: id.name.clone(),
            index: Some(id.index),
            ifaddrs: None,
        };
        let mut negative = NativeMeasurement::unavailable(ProbeError::Unsupported);
        negative.links = Sample::Value(vec![link(&spec.ordinary.interface)]);
        let mut present = negative.clone();
        present.links = Sample::Value(vec![link(&spec.ordinary.interface), link(&spec.interface)]);
        let denied = NativeMeasurement::unavailable(ProbeError::GetlinkSendDenied(13));
        let failed = NativeMeasurement::unavailable(ProbeError::Errno(13));
        let evaluate = |protected, raw, end, cleanup| {
            evaluate_link_request_denial(&spec, [&negative, &present, protected, end, cleanup], raw)
        };
        assert_eq!(
            evaluate(&denied, Some(&denied), &present, &negative),
            Some(LinkRequestDenial {
                raw_errno: 13,
                protected_errno: 13,
            })
        );
        assert_eq!(evaluate(&denied, None, &present, &negative), None);
        assert_eq!(evaluate(&failed, Some(&denied), &present, &negative), None);
        assert_eq!(evaluate(&denied, Some(&failed), &present, &negative), None);
        assert_eq!(evaluate(&denied, Some(&denied), &negative, &negative), None);
        assert_eq!(evaluate(&denied, Some(&denied), &present, &present), None);
        assert_eq!(evaluate(&present, Some(&denied), &present, &negative), None);
    }
}
