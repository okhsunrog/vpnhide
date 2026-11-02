use super::*;
use crate::fixtures::manifest;
#[test]
fn invalid_metadata_and_duplicates_fail_closed() {
    let valid = manifest();
    let mut cases = Vec::new();
    let mut m = valid.clone();
    m.schema = 2;
    cases.push(m);
    let mut m = valid.clone();
    m.payloads.push(m.payloads[0].clone());
    cases.push(m);
    let mut m = valid.clone();
    m.payloads[0].bytes = u64::MAX;
    cases.push(m);
    let mut m = valid.clone();
    m.payloads[0].sha256 = "no".into();
    cases.push(m);
    let mut m = valid.clone();
    m.payloads[0].control_version = 3;
    cases.push(m);
    let mut m = valid.clone();
    m.payloads[0].source_commit.clear();
    cases.push(m);
    let mut m = valid.clone();
    m.payloads[0].implementation = PayloadKind::Kmod {
        kmi: "../../outside".into(),
    };
    cases.push(m);
    for m in cases {
        assert!(m.validate().is_err(), "{m:?}");
    }
    let mut m = valid;
    m.payloads.pop();
    assert!(m.require_complete().is_err());
    assert!(Manifest::parse(&vec![b' '; MAX_MANIFEST_BYTES as usize + 1]).is_err());
    let json = serde_json::to_string(&manifest()).unwrap();
    assert!(Manifest::parse(json.replacen('{', "{\"unknown\":true,", 1).as_bytes()).is_err());
    assert!(Manifest::parse(json.replacen('{', "{\"schema\":1,", 1).as_bytes()).is_err());
}
#[test]
fn bounded_total_and_empty_files() {
    let mut m = manifest();
    m.payloads[0].bytes = 0;
    assert!(m.validate().is_err());
    for p in &mut m.payloads {
        p.bytes = MAX_PAYLOAD_BYTES;
    }
    assert!(m.validate().is_err());
}
