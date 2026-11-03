//! Closed-set proc interface reads. No writes, child processes or VPN gating.
use crate::proc_read::{ProcFile, read};
use std::collections::BTreeSet;
use vpnhide_diagnostic_model::{
    MAX_ADDRESSES, MAX_INTERFACES, NativeReadActor, ObservedAddress, ProbeError,
    ProcInterfaceActorMeasurement, ProcInterfaceMeasurement, Sample,
};

fn sample<T>(value: Result<T, ProbeError>) -> Sample<T> {
    match value {
        Ok(v) => Sample::Value(v),
        Err(e) => Sample::Error(e),
    }
}

fn label(value: &str) -> Result<String, ProbeError> {
    if value.is_empty()
        || value.len() > 15
        || !value.is_ascii()
        || value
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || b == b':')
    {
        return Err(ProbeError::Malformed);
    }
    Ok(value.to_owned())
}
fn devices(bytes: &[u8]) -> Result<Vec<String>, ProbeError> {
    let mut lines = std::str::from_utf8(bytes)
        .map_err(|_| ProbeError::Malformed)?
        .lines();
    let first = lines.next().ok_or(ProbeError::Malformed)?.trim_start();
    let second = lines.next().ok_or(ProbeError::Malformed)?.trim_start();
    if !first.starts_with("Inter-|")
        || !first.contains("Receive")
        || !first.contains("Transmit")
        || !second.starts_with("face |")
    {
        return Err(ProbeError::Malformed);
    }
    let mut names = BTreeSet::new();
    for line in lines {
        let (name, counts) = line.split_once(':').ok_or(ProbeError::Malformed)?;
        let name = label(name.trim())?;
        let fields: Vec<_> = counts.split_ascii_whitespace().collect();
        if fields.len() != 16
            || fields.iter().any(|f| f.parse::<u64>().is_err())
            || !names.insert(name)
        {
            return Err(ProbeError::Malformed);
        }
        if names.len() > MAX_INTERFACES {
            return Err(ProbeError::TooLarge);
        }
    }
    Ok(names.into_iter().collect())
}
fn ipv6(bytes: &[u8]) -> Result<Vec<ObservedAddress>, ProbeError> {
    let mut entries = BTreeSet::new();
    for line in std::str::from_utf8(bytes)
        .map_err(|_| ProbeError::Malformed)?
        .lines()
    {
        let fields: Vec<_> = line.split_ascii_whitespace().collect();
        if fields.len() != 6 || fields[0].len() != 32 || !fields[0].is_ascii() {
            return Err(ProbeError::Malformed);
        }
        let number =
            |i: usize| u32::from_str_radix(fields[i], 16).map_err(|_| ProbeError::Malformed);
        let index = number(1)?;
        let prefix = number(2)?;
        if index == 0 || index > i32::MAX as u32 || prefix > 128 || number(3)? > 255 {
            return Err(ProbeError::Malformed);
        }
        // IFA_F_* is full width (NOPREFIXROUTE alone is 0x200).
        number(4)?;
        let address = (0..16)
            .map(|i| {
                u8::from_str_radix(&fields[0][i * 2..i * 2 + 2], 16)
                    .map_err(|_| ProbeError::Malformed)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !entries.insert(ObservedAddress {
            name: Some(label(fields[5])?),
            index: Some(index),
            address,
            prefix: Some(prefix as u8),
        }) {
            return Err(ProbeError::Malformed);
        }
        if entries.len() > MAX_ADDRESSES {
            return Err(ProbeError::TooLarge);
        }
    }
    Ok(entries.into_iter().collect())
}
pub fn observe() -> ProcInterfaceMeasurement {
    ProcInterfaceMeasurement {
        actors: NativeReadActor::ALL
            .into_iter()
            .map(|actor| ProcInterfaceActorMeasurement {
                actor,
                devices: sample(read(ProcFile::InterfaceStats, actor).and_then(|b| devices(&b))),
                ipv6: sample(read(ProcFile::Ipv6Addresses, actor).and_then(|b| ipv6(&b))),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_proc_records_keep_names_addresses_and_strict_fields() {
        let prefix = "Inter-| Receive | Transmit\n face | bytes packets\n";
        let line = "lo: 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15\n";
        assert_eq!(
            devices(format!("{prefix}{line}").as_bytes()),
            Ok(vec!["lo".into()])
        );
        for text in [
            line.to_owned(),
            format!("{prefix}{line}{line}"),
            format!("{prefix}lo: 1\n"),
            format!("{prefix}tun0: x 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15\n"),
        ] {
            assert!(devices(text.as_bytes()).is_err());
        }
        let row = b"00000000000000000000000000000001 01 80 10 80 lo\n";
        assert!(ipv6(b"fd76706e686964650000000000010001 2a 80 00 282 tun000000000001\n").is_ok());
        let parsed = ipv6(row).unwrap();
        assert_eq!(parsed[0].name.as_deref(), Some("lo"));
        assert_eq!(parsed[0].index, Some(1));
        assert_eq!(parsed[0].prefix, Some(128));
        assert_eq!(parsed[0].address, [vec![0; 15], vec![1]].concat());
        for text in [b"".as_slice(), row] {
            assert!(ipv6(text).is_ok());
        }
        for text in [
            b"0000 01 80 10 80 lo\n".as_slice(),
            b"00000000000000000000000000000001 00 80 10 80 lo\n",
            b"00000000000000000000000000000001 01 81 10 80 lo\n",
        ] {
            assert!(ipv6(text).is_err());
        }
    }
    #[test]
    fn real_host_loopback_device_is_visible_to_both_read_actors() {
        let measured = observe();
        assert!(measured.bounded());
        for actor in measured.actors {
            assert!(
                actor
                    .devices
                    .value()
                    .expect("real host proc reference")
                    .iter()
                    .any(|n| n == "lo")
            );
        }
    }
}
