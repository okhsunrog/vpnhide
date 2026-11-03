use vpnhide_checks::netlink_wire::*;
use vpnhide_diagnostic_model::ProbeError;

fn frame(kind: u16, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; 16];
    bytes[4..6].copy_from_slice(&kind.to_ne_bytes());
    bytes[8..12].copy_from_slice(&1u32.to_ne_bytes());
    bytes.extend_from_slice(body);
    let length = bytes.len() as u32;
    bytes[..4].copy_from_slice(&length.to_ne_bytes());
    bytes.resize((bytes.len() + 3) & !3, 0);
    bytes
}
#[test]
fn failed_or_interrupted_dump_cannot_resume_into_empty_success() {
    for bytes in [vec![1], frame(20, &[0]), frame(3, &(-12i32).to_ne_bytes())] {
        let mut dump = Dump::new(1, 16);
        assert!(dump.consume(&bytes, |_| Ok(())).is_err());
        assert!(dump.consume(&frame(3, &[]), |_| Ok(())).is_err());
    }
    let mut interrupted = frame(16, &[1]);
    interrupted[6..8].copy_from_slice(&0x10u16.to_ne_bytes());
    assert!(Dump::new(1, 16).consume(&interrupted, |_| Ok(())).is_err());
    let trailing = [frame(3, &[]), frame(16, &[1])].concat();
    assert!(Dump::new(1, 16).consume(&trailing, |_| Ok(())).is_err());
}

#[test]
fn dump_sequence_and_negative_errno_are_independent_from_a_terminal_empty_result() {
    let mut wrong_sequence = frame(3, &[]);
    wrong_sequence[8..12].copy_from_slice(&2u32.to_ne_bytes());
    assert_eq!(
        Dump::new(1, 16).consume(&wrong_sequence, |_| Ok(())),
        Err(ProbeError::Malformed)
    );
    let rejected = frame(2, &(-libc::EPERM).to_ne_bytes());
    assert_eq!(
        Dump::new(1, 16).consume(&rejected, |_| Ok(())),
        Err(ProbeError::Errno(libc::EPERM))
    );
    // A successful ACK is not the end of a multipart dump.
    assert_eq!(
        Dump::new(1, 16).consume(&frame(2, &0i32.to_ne_bytes()), |_| Ok(())),
        Err(ProbeError::Malformed)
    );
}
#[test]
fn ack_binds_both_headers_and_preserves_negative_errno() {
    let request = frame(20, &[]);
    let body = [0i32.to_ne_bytes().as_slice(), request.as_slice()].concat();
    let ack = frame(2, &body);
    assert_eq!(acknowledgement(&ack, &request), Ok(()));
    let mut other = request.clone();
    other[8..12].copy_from_slice(&2u32.to_ne_bytes());
    assert!(acknowledgement(&ack, &other).is_err());
    assert!(acknowledgement(&ack, &frame(22, &[])).is_err());
    let negative = [(-13i32).to_ne_bytes().as_slice(), request.as_slice()].concat();
    assert_eq!(
        acknowledgement(&frame(2, &negative), &request),
        Err(ProbeError::Errno(13))
    );
    for length in 0..ack.len() {
        assert!(acknowledgement(&ack[..length], &request).is_err());
    }
}
#[test]
fn attributes_and_names_require_complete_bounds_and_one_terminator() {
    assert!(attributes(&[3, 0, 1, 0]).is_err());
    assert!(attributes(&[9, 0, 1, 0]).is_err());
    assert!(interface_name(b"lo\0tail\0").is_err());
    assert!(interface_name(b"lo").is_err());
    assert_eq!(interface_name(b"lo\0"), Ok("lo"));
}

#[test]
fn unaligned_rule_fields_and_malformed_trailing_attributes_use_shared_wire_bounds() {
    let mut body = vec![0u8; 12]; // fib_rule_hdr
    body.extend_from_slice(&8u16.to_ne_bytes());
    body.extend_from_slice(&15u16.to_ne_bytes()); // FRA_TABLE
    body.extend_from_slice(&0x0102_0304u32.to_ne_bytes());
    let mut unaligned = vec![0xff];
    unaligned.extend_from_slice(&frame(32, &body)); // RTM_NEWRULE
    let mut table = None;
    Dump::new(1, 32)
        .consume(&unaligned[1..], |record| {
            let attrs = attributes(&record[12..])?;
            assert_eq!(attrs[0].kind, 15);
            table = Some(u32_at(attrs[0].value, 0)?);
            Ok(())
        })
        .unwrap();
    assert_eq!(table, Some(0x0102_0304));

    body.extend_from_slice(&[3, 0, 15, 0]); // impossible attribute length
    let mut visited = 0;
    assert!(
        Dump::new(1, 32)
            .consume(&frame(32, &body), |record| {
                for _ in attributes(&record[12..])? {
                    visited += 1;
                }
                Ok(())
            })
            .is_err()
    );
    assert_eq!(
        visited, 0,
        "no partially validated attributes escape the shared parser"
    );
}
