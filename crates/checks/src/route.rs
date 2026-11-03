//! Closed read-only route collectors. Literal valid rows survive later errors;
//! collectors never infer hiding, qualify a stimulus or produce PASS.
use crate::{
    netlink_read::{self, DumpKind},
    netlink_wire as wire,
    proc_read::{self, ProcFile},
};
use vpnhide_diagnostic_model::{
    MAX_ROUTES, ObservedRoute, ProbeError, RouteCheck, RouteMeasurement, RouteObservation,
    RouteSurface,
};

fn malformed<T>() -> Result<T, ProbeError> {
    Err(ProbeError::Malformed)
}
fn push(
    rows: &mut Vec<ObservedRoute>,
    row: ObservedRoute,
    surface: RouteSurface,
) -> Result<(), ProbeError> {
    if !row.valid(surface) {
        return malformed();
    }
    if rows.len() >= MAX_ROUTES {
        return Err(ProbeError::TooLarge);
    }
    rows.push(row);
    Ok(())
}
struct RouteDump {
    wire: wire::Dump,
    rows: Vec<ObservedRoute>,
    surface: RouteSurface,
}
impl RouteDump {
    fn new(surface: RouteSurface) -> Self {
        Self {
            wire: wire::Dump::new(1, 24),
            rows: Vec::new(),
            surface,
        }
    }
    fn consume(&mut self, bytes: &[u8]) -> Result<bool, ProbeError> {
        let surface = self.surface;
        let rows = &mut self.rows;
        self.wire.consume(bytes, |body| {
            push(rows, netlink_row(body, surface)?, surface)
        })
    }
}
fn netlink_row(body: &[u8], surface: RouteSurface) -> Result<ObservedRoute, ProbeError> {
    if !surface.netlink() || body.len() < 12 {
        return malformed();
    }
    let length = surface.address_bytes();
    let family = if length == 4 {
        libc::AF_INET
    } else {
        libc::AF_INET6
    };
    if body[0] as i32 != family || body[1] as usize > length * 8 || body[2] as usize > length * 8 {
        return malformed();
    }
    let (mut destination, mut source, mut output, mut table, mut metric, mut gateway) =
        (None, None, None, None, None, None);
    for attribute in wire::attributes(&body[12..])? {
        match attribute.kind {
            1 | 2 | 5 => {
                if attribute.value.len() != length {
                    return malformed();
                }
                let field = match attribute.kind {
                    1 => &mut destination,
                    2 => &mut source,
                    _ => &mut gateway,
                };
                if field.replace(attribute.value.to_vec()).is_some() {
                    return malformed();
                }
            }
            4 | 6 | 15 => {
                if attribute.value.len() != 4 {
                    return malformed();
                }
                let field = match attribute.kind {
                    4 => &mut output,
                    6 => &mut metric,
                    _ => &mut table,
                };
                if field.replace(wire::u32_at(attribute.value, 0)?).is_some() {
                    return malformed();
                }
            }
            _ => {}
        }
    }
    let destination = match destination {
        Some(v) => v,
        None if body[1] == 0 => vec![0; length],
        None => return malformed(),
    };
    let row = ObservedRoute {
        destination,
        prefix: body[1],
        source,
        source_prefix: body[2],
        output_index: output,
        output_name: None,
        table: Some(table.unwrap_or(u32::from(body[4]))),
        protocol: Some(body[5]),
        scope: Some(body[6]),
        kind: Some(body[7]),
        metric: metric.unwrap_or(0),
        gateway,
        flags: wire::u32_at(body, 8)?,
    };
    if !row.valid(surface) {
        return malformed();
    }
    Ok(row)
}
fn hex(value: &str, length: usize) -> Result<u32, ProbeError> {
    if value.len() != length || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
        return malformed();
    }
    u32::from_str_radix(value, 16).map_err(|_| ProbeError::Malformed)
}
fn ipv6(value: &str) -> Result<Vec<u8>, ProbeError> {
    if value.len() != 32 || !value.is_ascii() {
        return malformed();
    }
    (0..16)
        .map(|i| hex(&value[i * 2..i * 2 + 2], 2).map(|v| v as u8))
        .collect()
}
fn proc_row(line: &str, surface: RouteSurface) -> Result<ObservedRoute, ProbeError> {
    let f: Vec<_> = line.split_ascii_whitespace().collect();
    match surface {
        RouteSurface::ProcIpv4 => {
            if f.len() != 11 {
                return malformed();
            }
            // proc IPv4 prints native u32 values, unlike the network-byte IPv6
            // text. Preserve that ABI rather than guessing a libc structure.
            let address = |index| hex(f[index], 8).map(|v| v.to_ne_bytes().to_vec());
            let mask: [u8; 4] = address(7)?.try_into().map_err(|_| ProbeError::Malformed)?;
            let mask = u32::from_be_bytes(mask);
            let prefix = mask.leading_ones();
            if mask != u32::MAX.checked_shl(32 - prefix).unwrap_or(0) {
                return malformed();
            }
            for i in [4, 5, 8, 9, 10] {
                f[i].parse::<u64>().map_err(|_| ProbeError::Malformed)?;
            }
            Ok(ObservedRoute {
                destination: address(1)?,
                prefix: prefix as u8,
                source: None,
                source_prefix: 0,
                output_index: None,
                output_name: Some(f[0].into()),
                table: None,
                protocol: None,
                scope: None,
                kind: None,
                metric: f[6].parse().map_err(|_| ProbeError::Malformed)?,
                gateway: Some(address(2)?),
                flags: if (4..=8).contains(&f[3].len()) {
                    hex(f[3], f[3].len())?
                } else {
                    return malformed();
                },
            })
        }
        RouteSurface::ProcIpv6 => {
            if f.len() != 10 {
                return malformed();
            }
            hex(f[6], 8)?;
            hex(f[7], 8)?;
            Ok(ObservedRoute {
                destination: ipv6(f[0])?,
                prefix: u8::try_from(hex(f[1], 2)?).map_err(|_| ProbeError::Malformed)?,
                source: Some(ipv6(f[2])?),
                source_prefix: u8::try_from(hex(f[3], 2)?).map_err(|_| ProbeError::Malformed)?,
                output_index: None,
                output_name: Some(f[9].into()),
                table: None,
                protocol: None,
                scope: None,
                kind: None,
                metric: hex(f[5], 8)?,
                gateway: Some(ipv6(f[4])?),
                flags: hex(f[8], 8)?,
            })
        }
        _ => malformed(),
    }
}
fn proc_rows(
    bytes: &[u8],
    surface: RouteSurface,
    rows: &mut Vec<ObservedRoute>,
) -> Result<(), ProbeError> {
    let mut lines = bytes.split_inclusive(|b| *b == b'\n').map(|line| {
        std::str::from_utf8(line)
            .map(|v| v.trim_end_matches(['\r', '\n']))
            .map_err(|_| ProbeError::Malformed)
    });
    if surface == RouteSurface::ProcIpv4 {
        let header: Vec<_> = lines
            .next()
            .ok_or(ProbeError::Malformed)??
            .split_ascii_whitespace()
            .collect();
        if header
            != [
                "Iface",
                "Destination",
                "Gateway",
                "Flags",
                "RefCnt",
                "Use",
                "Metric",
                "Mask",
                "MTU",
                "Window",
                "IRTT",
            ]
        {
            return malformed();
        }
    }
    for line in lines {
        push(rows, proc_row(line?, surface)?, surface)?;
    }
    Ok(())
}
pub fn observe() -> RouteMeasurement {
    RouteMeasurement {
        observations: RouteCheck::required()
            .into_iter()
            .map(|check| {
                let (routes, error) = match check.surface {
                    RouteSurface::NetlinkIpv4 | RouteSurface::NetlinkIpv6 => {
                        let mut dump = RouteDump::new(check.surface);
                        let kind = if check.surface == RouteSurface::NetlinkIpv4 {
                            DumpKind::RoutesIpv4
                        } else {
                            DumpKind::RoutesIpv6
                        };
                        let error =
                            netlink_read::dump(kind, check.actor, |bytes| dump.consume(bytes))
                                .err();
                        (dump.rows, error)
                    }
                    RouteSurface::ProcIpv4 | RouteSurface::ProcIpv6 => {
                        let file = if check.surface == RouteSurface::ProcIpv4 {
                            ProcFile::RoutesIpv4
                        } else {
                            ProcFile::RoutesIpv6
                        };
                        let read = proc_read::read_partial(file, check.actor);
                        let mut rows = Vec::new();
                        let error = proc_rows(&read.bytes, check.surface, &mut rows).err();
                        (rows, read.error.or(error))
                    }
                };
                RouteObservation {
                    check,
                    routes,
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
    fn literal_proc_formats_retain_route_identity_and_reject_bad_masks() {
        let row = proc_row(
            "tun0123456789ab 010212C6 00000000 0001 0 0 54321 FFFFFFFF 0 0 0",
            RouteSurface::ProcIpv4,
        )
        .unwrap();
        assert_eq!(row.destination, [198, 18, 2, 1]);
        assert_eq!(row.prefix, 32);
        assert_eq!(row.metric, 54321);
        assert_eq!(row.output_name.as_deref(), Some("tun0123456789ab"));
        assert_eq!(row.output_index, None);
        assert_eq!(row.table, None);
        assert!(
            proc_row(
                "lo 00000000 00000000 0001 0 0 1 00FF00FF 0 0 0",
                RouteSurface::ProcIpv4
            )
            .is_err()
        );
        let v6 = proc_row("fd000000000000000000000000000001 80 00000000000000000000000000000000 00 00000000000000000000000000000000 0000d431 00000000 00000000 00000001 vhr0123456789ab",RouteSurface::ProcIpv6).unwrap();
        assert_eq!(v6.destination.len(), 16);
        assert_eq!(v6.destination[0], 0xfd);
        assert_eq!(v6.prefix, 128);
        assert_eq!(v6.metric, 54321);
    }
    fn attribute(kind: u16, value: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&((value.len() + 4) as u16).to_ne_bytes());
        v.extend_from_slice(&kind.to_ne_bytes());
        v.extend_from_slice(value);
        v.resize((v.len() + 3) & !3, 0);
        v
    }
    fn body() -> Vec<u8> {
        let mut b = vec![2, 32, 0, 0, 254, 242, 253, 1, 0, 0, 0, 0];
        for (kind, value) in [
            (1, vec![198, 18, 2, 1]),
            (4, 7u32.to_ne_bytes().to_vec()),
            (6, 54321u32.to_ne_bytes().to_vec()),
        ] {
            b.extend_from_slice(&attribute(kind, &value));
        }
        b
    }
    #[test]
    fn netlink_requires_exact_family_attribute_width_and_unique_identity_fields() {
        let mut b = body();
        let row = netlink_row(&b, RouteSurface::NetlinkIpv4).unwrap();
        assert_eq!(row.output_index, Some(7));
        assert_eq!(row.protocol, Some(242));
        assert_eq!(row.metric, 54321);
        assert_eq!(row.output_name, None);
        b.extend_from_slice(&attribute(4, &8u32.to_ne_bytes()));
        assert_eq!(
            netlink_row(&b, RouteSurface::NetlinkIpv4),
            Err(ProbeError::Malformed)
        );
        assert_eq!(
            netlink_row(&body(), RouteSurface::NetlinkIpv6),
            Err(ProbeError::Malformed)
        );
        let mut b = body();
        b.extend_from_slice(&attribute(5, &[1, 2, 3]));
        assert_eq!(
            netlink_row(&b, RouteSurface::NetlinkIpv4),
            Err(ProbeError::Malformed)
        );
    }
    fn frame(kind: u16, body: &[u8]) -> Vec<u8> {
        let mut b = vec![0; 16];
        b[..4].copy_from_slice(&((16 + body.len()) as u32).to_ne_bytes());
        b[4..6].copy_from_slice(&kind.to_ne_bytes());
        b[8..12].copy_from_slice(&1u32.to_ne_bytes());
        b.extend_from_slice(body);
        b.resize((b.len() + 3) & !3, 0);
        b
    }
    #[test]
    fn later_dump_or_proc_failure_preserves_already_observed_route_rows() {
        let mut dump = RouteDump::new(RouteSurface::NetlinkIpv4);
        assert_eq!(dump.consume(&frame(24, &body())), Ok(false));
        assert_eq!(dump.rows.len(), 1);
        assert_eq!(
            dump.consume(&frame(3, &(-libc::EINTR).to_ne_bytes())),
            Err(ProbeError::Malformed)
        );
        assert_eq!(dump.rows.len(), 1);
        let mut batch = frame(24, &body());
        batch.extend_from_slice(&[1, 2, 3, 4]);
        let mut same_batch = RouteDump::new(RouteSurface::NetlinkIpv4);
        assert_eq!(same_batch.consume(&batch), Err(ProbeError::Malformed));
        assert_eq!(same_batch.rows.len(), 1);
        let mut rows = Vec::new();
        let raw=b"Iface Destination Gateway Flags RefCnt Use Metric Mask MTU Window IRTT\ntun0123456789ab 010212C6 00000000 0001 0 0 54321 FFFFFFFF 0 0 0\ntruncated";
        assert_eq!(
            proc_rows(raw, RouteSurface::ProcIpv4, &mut rows),
            Err(ProbeError::Malformed)
        );
        assert_eq!(rows.len(), 1);
        let mut invalid_utf8 = raw[..raw.len() - 9].to_vec();
        invalid_utf8.extend_from_slice(&[0xff]);
        let mut rows = Vec::new();
        assert_eq!(
            proc_rows(&invalid_utf8, RouteSurface::ProcIpv4, &mut rows),
            Err(ProbeError::Malformed)
        );
        assert_eq!(rows.len(), 1);
    }
}
