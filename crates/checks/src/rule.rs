//! Bounded RTM_GETRULE dumps. No rule writes are available to the Android worker.
use crate::{
    netlink_read::{self, DumpKind},
    netlink_wire as wire,
};
use std::time::{Duration, Instant};
use vpnhide_diagnostic_model::{
    MAX_ROUTES, ObservedRule, ProbeError, RouteCheck, RouteMeasurement, RouteObservation,
    RouteSurface,
};

fn parse(body: &[u8], family: u8) -> Result<ObservedRule, ProbeError> {
    if body.len() < 12 || body[0] != family {
        return Err(ProbeError::Malformed);
    }
    let mut row = ObservedRule {
        priority: None,
        goto: None,
        output_name: None,
        uid_range: None,
        table: u32::from(body[4]),
        protocol: None,
        action: body[7],
        flags: wire::u32_at(body, 8)?,
        additional_selectors: body[1..4] != [0; 3] || body[5..7] != [0; 2],
    };
    let mut seen = std::collections::BTreeSet::new();
    for attr in wire::attributes(&body[12..])? {
        let value = attr.value;
        if attr.kind != 18 && !seen.insert(attr.kind) {
            return Err(ProbeError::Malformed);
        }
        let word = || {
            if value.len() != 4 {
                return Err(ProbeError::Malformed);
            }
            wire::u32_at(value, 0)
        };
        match attr.kind {
            5 => row.goto = Some(word()?),
            6 => row.priority = Some(word()?),
            15 => row.table = word()?,
            17 => {
                let name = value.strip_suffix(&[0]).ok_or(ProbeError::Malformed)?;
                if name.is_empty() || name.len() > 15 || !name.iter().all(u8::is_ascii_graphic) {
                    return Err(ProbeError::Malformed);
                }
                row.output_name = Some(
                    std::str::from_utf8(name)
                        .map_err(|_| ProbeError::Malformed)?
                        .into(),
                );
            }
            20 if value.len() == 8 => {
                let lo = wire::u32_at(value, 0)?;
                let hi = wire::u32_at(value, 4)?;
                if lo > hi {
                    return Err(ProbeError::Malformed);
                }
                row.uid_range = Some([lo, hi]);
            }
            21 if value.len() == 1 => row.protocol = Some(value[0]),
            20 | 21 => return Err(ProbeError::Malformed),
            13 | 14 => row.additional_selectors |= word()? != u32::MAX,
            18 if value.is_empty() => (),
            _ => row.additional_selectors = true,
        }
    }
    Ok(row)
}

pub fn observe() -> RouteMeasurement {
    observe_before(Instant::now() + Duration::from_secs(1))
}

pub(crate) fn observe_before(deadline: Instant) -> RouteMeasurement {
    RouteMeasurement {
        observations: RouteCheck::rules()
            .map(|check| {
                let (kind, family) = if check.surface == RouteSurface::RuleIpv4 {
                    (DumpKind::RulesIpv4, libc::AF_INET as u8)
                } else {
                    (DumpKind::RulesIpv6, libc::AF_INET6 as u8)
                };
                let mut dump = wire::Dump::new(1, 32); // RTM_NEWRULE replies
                let mut rules = Vec::new();
                let error = netlink_read::dump_before(kind, check.actor, deadline, |bytes| {
                    dump.consume(bytes, |body| {
                        if rules.len() == MAX_ROUTES {
                            return Err(ProbeError::TooLarge);
                        }
                        rules.push(parse(body, family)?);
                        Ok(())
                    })
                })
                .err();
                RouteObservation {
                    check,
                    routes: Vec::new(),
                    rules,
                    error,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_priority_and_retains_unrecognized_selectors() {
        let mut body = vec![2, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0];
        body.extend_from_slice(&8u16.to_ne_bytes());
        body.extend_from_slice(&6u16.to_ne_bytes());
        body.extend_from_slice(&40u32.to_ne_bytes());
        assert_eq!(parse(&body, 2).unwrap().priority, Some(40));
        body.extend_from_slice(&[4, 0, 3, 0]); // FRA_IIFNAME is never generated
        assert!(parse(&body, 2).unwrap().additional_selectors);
        body.extend_from_within(12..20);
        assert_eq!(parse(&body, 2), Err(ProbeError::Malformed));
    }
}
