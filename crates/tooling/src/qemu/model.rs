//! Pure QEMU test policy. A complete declared plan, positive baselines and
//! independent preservation controls determine success, not an emitted summary.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
pub const REPORT_SCHEMA: u32 = 1;
pub const TARGET_UID: u32 = 10000;
pub const BASELINE_UID: u32 = 10002;
pub const INET_GID: u32 = 3003;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Kmod,
    Builtin,
    Kpm,
    Userspace,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Full,
    Preflight,
    CorePreflight,
    NativeUnit,
    FixtureRaw,
    FixtureProtected,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Raw,
    Protected,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub present: bool,
    pub required: bool,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probes {
    pub gai: Probe,
    pub ifconf: Probe,
    pub bind: Probe,
    pub ioctl: Probe,
    pub compat: Probe,
    pub preflight: Probe,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: u32,
    pub session: String,
    pub backend: Backend,
    pub mode: Mode,
    pub payload_sha256: Option<String>,
    pub probes: Probes,
    #[serde(default)]
    pub kpm_phase: Option<Phase>,
}
pub const HIDE: [&str; 15] = [
    "getifaddrs",
    "siocgifconf",
    "dev_ioctl",
    "proc_route_v4",
    "proc_route_v6",
    "netlink_route4",
    "hostroute4",
    "netlink_route6",
    "hostroute6",
    "policy_rule",
    "sysfs_stat",
    "sysfs_readdir",
    "sysfs_open",
    "proc_sys_stat",
    "proc_sys_readdir",
];
pub const KEEP: [&str; 8] = [
    "keep_proc_route_v4",
    "keep_getifaddrs",
    "keep_siocgifconf",
    "keep_dev_ioctl",
    "keep_netlink_route4",
    "keep_policy_rule",
    "keep_sysfs_readdir",
    "keep_proc_sys_readdir",
];
pub const BIND: [&str; 7] = [
    "bind_device_raw",
    "bind_device_nul",
    "bind_bad_pointer",
    "bind_bad_length",
    "bind_ifindex",
    "keep_bind_device",
    "bind_absent_name",
];
fn invalid(s: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s.to_string())
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Plan {
    pub fn validate(&self) -> io::Result<()> {
        if self.schema != REPORT_SCHEMA
            || !digest(&self.session)
            || self.payload_sha256.as_ref().is_some_and(|s| !digest(s))
        {
            return Err(invalid("invalid harness schema/session/payload identity"));
        }
        if self.backend == Backend::Builtin && self.payload_sha256.is_some() {
            return Err(invalid("builtin does not load a payload"));
        }
        if self.backend != Backend::Builtin && self.payload_sha256.is_none() {
            return Err(invalid("payload identity is required"));
        }
        if (self.mode == Mode::NativeUnit) != (self.backend == Backend::Userspace) {
            return Err(invalid(
                "native unit scope requires the userspace suite backend",
            ));
        }
        if !matches!(self.mode, Mode::Full | Mode::NativeUnit) && self.backend != Backend::Kmod {
            return Err(invalid(
                "fixture/preflight modes require the isolated ko backend",
            ));
        }
        if self.kpm_phase.is_some() && self.backend != Backend::Kpm {
            return Err(invalid("KPM phase on a different backend"));
        }
        if self.probes.compat.present && !self.probes.bind.present {
            return Err(invalid(
                "compat probe needs the paired native binding controls",
            ));
        }
        for probe in [
            self.probes.gai,
            self.probes.ifconf,
            self.probes.bind,
            self.probes.ioctl,
            self.probes.compat,
            self.probes.preflight,
        ] {
            if probe.required && !probe.present {
                return Err(invalid("required native probe unavailable"));
            }
        }
        if matches!(self.mode, Mode::Preflight | Mode::CorePreflight)
            && !self.probes.preflight.present
        {
            return Err(invalid("preflight mode needs its syscall oracle"));
        }
        Ok(())
    }
    /// Every expected identity must be emitted. `true` permits an explicit skip
    /// only because its optional host probe was absent before the guest booted.
    pub fn expected(&self) -> io::Result<BTreeMap<String, bool>> {
        self.validate()?;
        let mut result = BTreeMap::new();
        if self.mode == Mode::NativeUnit {
            result.insert("native_unit_suite".into(), false);
            return Ok(result);
        }
        if matches!(self.mode, Mode::Preflight | Mode::CorePreflight) {
            result.insert("loader_preflight".into(), false);
            if self.mode == Mode::CorePreflight {
                result.insert("core_preflight_dispatch".into(), false);
            }
            return Ok(result);
        }
        if matches!(self.mode, Mode::FixtureRaw | Mode::FixtureProtected) {
            result.insert("native_fixture".into(), false);
            if self.mode == Mode::FixtureProtected && self.probes.preflight.present {
                result.insert("loader_preflight".into(), false);
            }
            return Ok(result);
        }
        for name in HIDE.into_iter().chain(KEEP) {
            result.insert(name.into(), false);
        }
        for name in ["gai_getifaddrs", "keep_gai_getifaddrs"] {
            result.insert(name.into(), !self.probes.gai.present);
        }
        for name in BIND {
            result.insert(name.into(), !self.probes.bind.present);
        }
        for name in ["ioctl_hwaddr", "ioctl_addr"] {
            result.insert(name.into(), !self.probes.ioctl.present);
        }
        result.insert(
            if self.backend == Backend::Kpm {
                "ifconf_tail"
            } else {
                "ifconf_probe"
            }
            .into(),
            !self.probes.ifconf.present,
        );
        if self.backend == Backend::Kpm {
            result.insert("ifconf_probe".into(), !self.probes.ifconf.present);
        }
        if self.backend != Backend::Kpm {
            result.insert("filesystem_stats".into(), false);
            result.insert("permanent_module".into(), false);
            result.insert("bind_device_compat".into(), !self.probes.compat.present);
            if self.backend == Backend::Kmod && self.probes.preflight.present {
                result.insert("loader_preflight".into(), false);
            }
        }
        Ok(result)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    NativeCovered,
    NotApplicable,
    Skip,
    Fail,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultRow {
    pub vector: String,
    pub verdict: Verdict,
    pub detail: String,
}
impl ResultRow {
    pub fn new(name: &str, verdict: Verdict, detail: impl Into<String>) -> Self {
        Self {
            vector: name.into(),
            verdict,
            detail: detail.into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: u32,
    pub session: String,
    pub backend: Backend,
    pub mode: Mode,
    pub payload_sha256: Option<String>,
    pub rows: Vec<ResultRow>,
    pub setup_ok: bool,
    pub loaded: bool,
    pub registered: u32,
}
impl Report {
    pub fn new(plan: &Plan) -> Self {
        Self {
            schema: REPORT_SCHEMA,
            session: plan.session.clone(),
            backend: plan.backend,
            mode: plan.mode,
            payload_sha256: plan.payload_sha256.clone(),
            rows: Vec::new(),
            setup_ok: false,
            loaded: false,
            registered: 0,
        }
    }
    pub fn validate(&self, plan: &Plan) -> io::Result<()> {
        let expected = plan.expected()?;
        if self.schema != REPORT_SCHEMA
            || self.session != plan.session
            || self.backend != plan.backend
            || self.mode != plan.mode
            || self.payload_sha256 != plan.payload_sha256
            || self.rows.len() != expected.len()
            || !self.setup_ok
        {
            return Err(invalid(
                "report does not complete the declared session/plan",
            ));
        }
        if matches!(
            self.mode,
            Mode::Preflight | Mode::CorePreflight | Mode::FixtureRaw | Mode::NativeUnit
        ) && (self.loaded || self.registered != 0)
        {
            return Err(invalid("unexpected backend exposure in a no-load plan"));
        }
        if !matches!(
            self.mode,
            Mode::Preflight | Mode::CorePreflight | Mode::FixtureRaw | Mode::NativeUnit
        ) && (!self.loaded
            || (self.mode == Mode::Full && self.backend != Backend::Kpm && self.registered == 0))
        {
            return Err(invalid("backend loading/registration was not confirmed"));
        }
        let mut seen = BTreeSet::new();
        for row in &self.rows {
            if !seen.insert(&row.vector) || row.detail.len() > 4096 {
                return Err(invalid("duplicate or excessive result"));
            }
            let skip = *expected
                .get(&row.vector)
                .ok_or_else(|| invalid("unplanned vector"))?;
            if matches!(row.verdict, Verdict::Fail | Verdict::Unknown)
                || (row.verdict == Verdict::Skip && !skip)
                || (skip && row.verdict != Verdict::Skip)
            {
                return Err(invalid(format!(
                    "unverified vector {}: {:?}",
                    row.vector, row.verdict
                )));
            }
            if row.verdict == Verdict::NotApplicable && row.vector != "bind_ifindex" {
                return Err(invalid("unsupported option applicability claim"));
            }
            if row.verdict == Verdict::NativeCovered
                && !matches!(
                    row.vector.as_str(),
                    "bind_device_raw"
                        | "bind_device_nul"
                        | "bind_ifindex"
                        | "keep_bind_device"
                        | "bind_device_compat"
                )
            {
                return Err(invalid("unsupported applicability claim"));
            }
        }
        Ok(())
    }
}
/// One process invocation's fields, parsed once. Missing, negative and duplicate
/// values cannot become a positive count or a successful zero observation.
pub fn fields(bytes: &[u8]) -> io::Result<BTreeMap<String, i64>> {
    if bytes.len() > 64 * 1024 {
        return Err(invalid("probe output exceeds budget"));
    }
    let text = std::str::from_utf8(bytes).map_err(invalid)?;
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| invalid("malformed probe field"))?;
        if key.is_empty()
            || key.len() > 128
            || !key
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            || fields.len() >= 128
            || fields
                .insert(key.into(), value.parse::<i64>().map_err(invalid)?)
                .is_some()
        {
            return Err(invalid("duplicate or invalid probe field"));
        }
    }
    Ok(fields)
}
#[derive(Debug, Clone, Copy)]
pub enum CountMode {
    Hide,
    KeepPositive,
    KeepExact,
}
pub fn count(name: &str, mode: CountMode, raw: Option<i64>, protected: Option<i64>) -> ResultRow {
    let detail = format!("raw={raw:?} protected={protected:?}");
    let (Some(raw), Some(protected)) = (raw, protected) else {
        return ResultRow::new(name, Verdict::Unknown, detail);
    };
    if raw <= 0 || protected < 0 {
        return ResultRow::new(
            name,
            Verdict::Unknown,
            format!("baseline missing or invalid: {detail}"),
        );
    }
    let pass = match mode {
        CountMode::Hide => protected == 0,
        CountMode::KeepPositive => protected > 0,
        CountMode::KeepExact => raw == protected,
    };
    ResultRow::new(
        name,
        if pass { Verdict::Pass } else { Verdict::Fail },
        detail,
    )
}
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    pub errno: i64,
    pub state: i64,
}
pub fn binding(fields: &BTreeMap<String, i64>, prefix: &str) -> Option<Binding> {
    Some(Binding {
        errno: *fields.get(&format!("{prefix}_ERRNO"))?,
        state: *fields.get(&format!("{prefix}_STATE"))?,
    })
}
pub fn hidden_bind(
    name: &str,
    raw: Option<Binding>,
    protected: Option<Binding>,
    absent: Option<Binding>,
) -> ResultRow {
    let detail = format!("raw={raw:?} protected={protected:?} absent={absent:?}");
    let (Some(raw), Some(protected), Some(absent)) = (raw, protected, absent) else {
        return ResultRow::new(name, Verdict::Unknown, detail);
    };
    if name == "bind_ifindex"
        && raw.errno == 92
        && raw.state == -92
        && protected.errno == 92
        && protected.state == -92
    {
        return ResultRow::new(
            name,
            Verdict::NotApplicable,
            "symmetric setsockopt/getsockopt ENOPROTOOPT",
        );
    }
    if raw.errno < 0
        || protected.errno < 0
        || absent.errno <= 0
        || absent.state != 0
        || raw.state < 0
        || protected.state < 0
    {
        return ResultRow::new(
            name,
            Verdict::Unknown,
            format!("baseline/readback unavailable: {detail}"),
        );
    }
    if raw.errno == 1 && raw.state == 0 && protected.errno == 1 && protected.state == 0 {
        return ResultRow::new(
            name,
            Verdict::NativeCovered,
            "symmetric CAP_NET_RAW denial and measured unbound state",
        );
    }
    ResultRow::new(
        name,
        if raw.errno == 0
            && raw.state == 1
            && protected.errno == absent.errno
            && protected.state == 0
        {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
        detail,
    )
}
pub fn preserved_bind(raw: Option<Binding>, protected: Option<Binding>) -> ResultRow {
    let name = "keep_bind_device";
    let detail = format!("raw={raw:?} protected={protected:?}");
    let (Some(raw), Some(protected)) = (raw, protected) else {
        return ResultRow::new(name, Verdict::Unknown, detail);
    };
    let verdict = if raw.errno < 0 || protected.errno < 0 || raw.state < 0 || protected.state < 0 {
        Verdict::Unknown
    } else if raw.errno == 0 && protected.errno == 0 && raw.state == 1 && protected.state == 1 {
        Verdict::Pass
    } else if raw.errno == 1 && protected.errno == 1 && raw.state == 0 && protected.state == 0 {
        Verdict::NativeCovered
    } else {
        Verdict::Fail
    };
    ResultRow::new(name, verdict, detail)
}
pub fn bad_bind(
    name: &str,
    raw: Option<Binding>,
    protected: Option<Binding>,
    same_errno: bool,
) -> ResultRow {
    let detail = format!("raw={raw:?} protected={protected:?}");
    let (Some(raw), Some(protected)) = (raw, protected) else {
        return ResultRow::new(name, Verdict::Unknown, detail);
    };
    let verdict = if raw.errno < 0 || protected.errno < 0 || raw.state < 0 || protected.state < 0 {
        Verdict::Unknown
    } else if raw.errno > 0
        && protected.errno > 0
        && raw.state == 0
        && protected.state == 0
        && (!same_errno || raw.errno == protected.errno)
    {
        Verdict::Pass
    } else {
        Verdict::Fail
    };
    ResultRow::new(name, verdict, detail)
}
pub fn absent_bind(value: Option<Binding>) -> ResultRow {
    ResultRow::new(
        "bind_absent_name",
        match value {
            None => Verdict::Unknown,
            Some(v) if v.errno < 0 || v.state < 0 => Verdict::Unknown,
            Some(v) if v.errno > 0 && v.state == 0 => Verdict::Pass,
            _ => Verdict::Fail,
        },
        format!("raw absent={value:?}"),
    )
}
pub fn ioctl(name: &str, raw: Option<i64>, protected: Option<i64>) -> ResultRow {
    let verdict = match (raw, protected) {
        (Some(0), Some(19)) => Verdict::Pass,
        (Some(r), Some(t)) if r >= 0 && t >= 0 => Verdict::Fail,
        _ => Verdict::Unknown,
    };
    ResultRow::new(
        name,
        verdict,
        format!("raw errno={raw:?} protected errno={protected:?}"),
    )
}
#[derive(Debug, Clone, Copy)]
pub struct Ifconf {
    pub size: i64,
    pub fill: i64,
    pub vpn: i64,
    pub tail: i64,
    pub tail_vpn: i64,
}
pub fn ifconf(fields: &BTreeMap<String, i64>) -> Option<Ifconf> {
    Some(Ifconf {
        size: *fields.get("IFCONF_SIZE")?,
        fill: *fields.get("IFCONF_FILL")?,
        vpn: *fields.get("IFCONF_FILL_VPN")?,
        tail: *fields.get("IFCONF_TAIL")?,
        tail_vpn: *fields.get("IFCONF_TAIL_VPN")?,
    })
}
pub fn ifconf_pair(name: &str, raw: Option<Ifconf>, protected: Option<Ifconf>) -> ResultRow {
    classify_ifconf(name, raw, protected, true)
}
/// Compaction, stale tail and ordinary-object preservation, independently of
/// the full size/fill check. This row cannot prove NULL-query concealment;
/// the required `ifconf_probe` row checks that separate behavior.
pub fn ifconf_tail(raw: Option<Ifconf>, protected: Option<Ifconf>) -> ResultRow {
    classify_ifconf("ifconf_tail", raw, protected, false)
}
fn classify_ifconf(
    name: &str,
    raw: Option<Ifconf>,
    protected: Option<Ifconf>,
    size_query: bool,
) -> ResultRow {
    let detail = format!(
        "scope={} raw={raw:?} protected={protected:?}",
        if size_query {
            "size/fill/tail"
        } else {
            "fill/tail only; size query NOT covered by this row"
        }
    );
    let (Some(r), Some(t)) = (raw, protected) else {
        return ResultRow::new(name, Verdict::Unknown, detail);
    };
    if [
        r.size, r.fill, r.vpn, r.tail, r.tail_vpn, t.size, t.fill, t.vpn, t.tail, t.tail_vpn,
    ]
    .iter()
    .any(|v| *v < 0)
        || r.vpn <= 0
        || r.fill <= r.vpn
    {
        return ResultRow::new(
            name,
            Verdict::Unknown,
            format!("positive VPN/ordinary baseline missing: {detail}"),
        );
    }
    ResultRow::new(
        name,
        if r.size == r.fill
            && (t.size == t.fill || (!size_query && t.size == r.size))
            && r.fill <= 64
            && t.fill == r.fill - r.vpn
            && t.vpn == 0
            && r.tail == 0
            && t.tail == 0
            && r.tail_vpn == 0
            && t.tail_vpn == 0
        {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
        detail,
    )
}
pub fn panic_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| {
            [
                "Unable to handle kernel",
                "Internal error: Oops",
                "kernel BUG at",
                "] BUG:",
                "Kernel panic",
            ]
            .iter()
            .any(|marker| line.contains(marker))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan() -> Plan {
        Plan {
            schema: REPORT_SCHEMA,
            session: "a".repeat(64),
            backend: Backend::Kmod,
            mode: Mode::Full,
            payload_sha256: Some("b".repeat(64)),
            probes: Probes::default(),
            kpm_phase: None,
        }
    }
    #[test]
    fn complete_plan_has_all_identities_and_missing_native_probes_remain_explicit() {
        let p = plan();
        assert_eq!(p.expected().unwrap().len(), 38);
        let mut k = p.clone();
        k.backend = Backend::Kpm;
        assert_eq!(k.expected().unwrap().len(), 36);
        let mut required = p.clone();
        required.probes.gai.required = true;
        assert!(required.expected().is_err());
        let mut r = Report::new(&p);
        r.setup_ok = true;
        r.loaded = true;
        r.registered = 1;
        r.rows = p
            .expected()
            .unwrap()
            .into_iter()
            .map(|(n, skip)| {
                ResultRow::new(
                    &n,
                    if skip { Verdict::Skip } else { Verdict::Pass },
                    "independent fixture result",
                )
            })
            .collect();
        r.validate(&p).unwrap();
        r.rows.pop();
        assert!(r.validate(&p).is_err());
    }
    #[test]
    fn summaries_or_duplicate_wrong_session_and_unknown_rows_cannot_pass() {
        let p = plan();
        let mut r = Report::new(&p);
        r.setup_ok = true;
        r.loaded = true;
        r.registered = 1;
        assert!(r.validate(&p).is_err());
        r.rows = p
            .expected()
            .unwrap()
            .into_iter()
            .map(|(n, skip)| {
                ResultRow::new(
                    &n,
                    if skip { Verdict::Skip } else { Verdict::Pass },
                    "fixture",
                )
            })
            .collect();
        r.rows[1] = r.rows[0].clone();
        assert!(r.validate(&p).is_err());
        r.session = "c".repeat(64);
        assert!(r.validate(&p).is_err());
    }
    #[test]
    fn negative_and_empty_observations_never_prove_hiding() {
        assert_eq!(
            count("x", CountMode::Hide, Some(0), Some(0)).verdict,
            Verdict::Unknown
        );
        assert_eq!(
            count("x", CountMode::Hide, Some(2), Some(0)).verdict,
            Verdict::Pass
        );
        assert_eq!(
            count("x", CountMode::Hide, Some(2), Some(1)).verdict,
            Verdict::Fail
        );
        assert_eq!(ioctl("x", Some(-1), Some(19)).verdict, Verdict::Unknown);
        assert!(fields(b"A=1\nA=0\n").is_err());
        assert!(fields(b"A=not-number\n").is_err());
    }
    #[test]
    fn failed_readback_is_not_native_coverage_and_ifindex_na_is_symmetric() {
        let b = |errno, state| Some(Binding { errno, state });
        assert_eq!(
            hidden_bind("bind_device_raw", b(1, -1), b(1, -1), b(1, 0)).verdict,
            Verdict::Unknown
        );
        assert_eq!(
            hidden_bind("bind_device_raw", b(1, 0), b(1, 0), b(1, 0)).verdict,
            Verdict::NativeCovered
        );
        assert_eq!(
            hidden_bind("bind_ifindex", b(92, -92), b(92, -92), b(19, 0)).verdict,
            Verdict::NotApplicable
        );
        assert_ne!(
            hidden_bind("bind_ifindex", b(0, 1), b(92, -92), b(19, 0)).verdict,
            Verdict::NotApplicable
        );
    }
    #[test]
    fn ifconf_requires_positive_same_ioctl_vpn_and_exact_ordinary_preservation() {
        let r = Ifconf {
            size: 3,
            fill: 3,
            vpn: 1,
            tail: 0,
            tail_vpn: 0,
        };
        let t = Ifconf {
            size: 2,
            fill: 2,
            vpn: 0,
            tail: 0,
            tail_vpn: 0,
        };
        assert_eq!(ifconf_pair("x", Some(r), Some(t)).verdict, Verdict::Pass);
        let kpm = Ifconf { size: r.size, ..t };
        assert_eq!(ifconf_pair("x", Some(r), Some(kpm)).verdict, Verdict::Fail);
        let tail = ifconf_tail(Some(r), Some(kpm));
        assert_eq!(tail.verdict, Verdict::Pass);
        assert!(tail.detail.contains("size query NOT covered"));
        assert_eq!(
            ifconf_pair(
                "x",
                Some(r),
                Some(Ifconf {
                    size: 0,
                    fill: 0,
                    ..t
                })
            )
            .verdict,
            Verdict::Fail
        );
        assert_eq!(
            ifconf_pair("x", Some(Ifconf { vpn: 0, ..r }), Some(t)).verdict,
            Verdict::Unknown
        );
    }
    #[test]
    fn panic_classifier_ignores_legacy_benign_substrings_but_scans_after_summary() {
        let log = "dynamic_debug: message\nunittest internal error: expected\nSUMMARY pass=38 fail=0\n[1] Internal error: Oops: 96000004\n";
        assert_eq!(panic_lines(log), vec!["[1] Internal error: Oops: 96000004"]);
    }
}

type NativeObservation = (String, Option<Vec<(String, i64)>>);

/// KPM load arguments are immutable for a boot. RAW and protected evidence must
/// come from separate guests, with the same declared probes and payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseReport {
    pub plan: Plan,
    pub boot: String,
    pub setup_ok: bool,
    pub loaded: bool,
    pub counts: Vec<(String, Option<i64>)>,
    pub fields: Vec<NativeObservation>,
}
impl PhaseReport {
    pub fn validate(&self, plan: &Plan) -> io::Result<()> {
        if serde_json::to_vec(&self.plan).map_err(invalid)?
            != serde_json::to_vec(plan).map_err(invalid)?
            || plan.kpm_phase.is_none()
            || self.boot.len() != 36
            || !self
                .boot
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b'-')
            || !self.setup_ok
            || !self.loaded
        {
            return Err(invalid("incomplete or stale KPM phase"));
        }
        let expected: BTreeSet<_> = HIDE.into_iter().chain(KEEP).map(str::to_string).collect();
        let actual: BTreeSet<_> = self.counts.iter().map(|(k, _)| k.clone()).collect();
        if actual != expected || actual.len() != self.counts.len() {
            return Err(invalid("missing/duplicate KPM observation"));
        }
        let native: BTreeSet<_> = [
            ("gai", plan.probes.gai),
            ("ifconf", plan.probes.ifconf),
            ("bind", plan.probes.bind),
            ("ioctl", plan.probes.ioctl),
        ]
        .into_iter()
        .filter(|(_, p)| p.present)
        .map(|(n, _)| n.to_string())
        .collect();
        let actual: BTreeSet<_> = self.fields.iter().map(|(k, _)| k.clone()).collect();
        if actual != native || actual.len() != self.fields.len() {
            return Err(invalid("missing/duplicate KPM native observation"));
        }
        for (_, rows) in &self.fields {
            if let Some(rows) = rows
                && (rows.len() > 128
                    || rows.iter().map(|(k, _)| k).collect::<BTreeSet<_>>().len() != rows.len())
            {
                return Err(invalid("duplicate/excessive native fields"));
            }
        }
        Ok(())
    }
    fn native(&self, name: &str) -> Option<BTreeMap<String, i64>> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)?
            .1
            .as_ref()
            .map(|rows| rows.iter().cloned().collect())
    }
}
pub fn compare_kpm(plan: &Plan, raw: &PhaseReport, target: &PhaseReport) -> io::Result<Report> {
    if plan.backend != Backend::Kpm || plan.kpm_phase.is_some() {
        return Err(invalid("expected aggregate KPM plan"));
    }
    let mut phase = plan.clone();
    phase.kpm_phase = Some(Phase::Raw);
    raw.validate(&phase)?;
    phase.kpm_phase = Some(Phase::Protected);
    target.validate(&phase)?;
    if raw.boot == target.boot {
        return Err(invalid("KPM phases reused the same boot"));
    }
    let mut report = Report::new(plan);
    report.setup_ok = true;
    report.loaded = true;
    for name in HIDE.into_iter().chain(KEEP) {
        let r = raw
            .counts
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| *v);
        let t = target
            .counts
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| *v);
        let mode = if HIDE.contains(&name) {
            CountMode::Hide
        } else if name == "keep_netlink_route4" {
            CountMode::KeepPositive
        } else {
            CountMode::KeepExact
        };
        report.rows.push(count(name, mode, r, t));
    }
    for (probe, present) in [
        ("gai", plan.probes.gai.present),
        ("ifconf", plan.probes.ifconf.present),
        ("bind", plan.probes.bind.present),
        ("ioctl", plan.probes.ioctl.present),
    ] {
        if !present {
            continue;
        }
        let r = raw.native(probe);
        let t = target.native(probe);
        let val = |f: &Option<BTreeMap<String, i64>>, k: &str| f.as_ref()?.get(k).copied();
        let bind = |f: &Option<BTreeMap<String, i64>>, k: &str| binding(f.as_ref()?, k);
        match probe {
            "gai" => {
                for (n, k, m) in [
                    ("gai_getifaddrs", "GAI_VPN0", CountMode::Hide),
                    ("keep_gai_getifaddrs", "GAI_OTHER", CountMode::KeepPositive),
                ] {
                    report.rows.push(count(n, m, val(&r, k), val(&t, k)));
                }
            }
            "ifconf" => {
                report.rows.push(ifconf_tail(
                    r.as_ref().and_then(ifconf),
                    t.as_ref().and_then(ifconf),
                ));
                report.rows.push(ifconf_pair(
                    "ifconf_probe",
                    r.as_ref().and_then(ifconf),
                    t.as_ref().and_then(ifconf),
                ));
            }
            "ioctl" => {
                for (n, k) in [
                    ("ioctl_hwaddr", "HWADDR_ERRNO"),
                    ("ioctl_addr", "ADDR_ERRNO"),
                ] {
                    report.rows.push(ioctl(n, val(&r, k), val(&t, k)));
                }
            }
            "bind" => {
                let absent = bind(&r, "BIND_ABSENT");
                for (n, k) in [
                    ("bind_device_raw", "BIND_NAME_RAW"),
                    ("bind_device_nul", "BIND_NAME_NUL"),
                    ("bind_ifindex", "BIND_INDEX"),
                ] {
                    report
                        .rows
                        .push(hidden_bind(n, bind(&r, k), bind(&t, k), absent));
                }
                report.rows.push(bad_bind(
                    "bind_bad_pointer",
                    bind(&r, "BIND_BADPTR"),
                    bind(&t, "BIND_BADPTR"),
                    false,
                ));
                report.rows.push(bad_bind(
                    "bind_bad_length",
                    bind(&r, "BIND_BADLEN"),
                    bind(&t, "BIND_BADLEN"),
                    true,
                ));
                report
                    .rows
                    .push(preserved_bind(bind(&r, "BIND_KEEP"), bind(&t, "BIND_KEEP")));
                report.rows.push(absent_bind(absent));
            }
            _ => unreachable!(),
        }
    }
    for (n, skip) in plan.expected()? {
        if skip {
            report.rows.push(ResultRow::new(
                &n,
                Verdict::Skip,
                "optional host probe absent",
            ));
        }
    }
    report.validate(plan)?;
    Ok(report)
}
#[cfg(test)]
mod phase_tests {
    use super::*;
    fn fixture() -> (Plan, PhaseReport, PhaseReport) {
        let plan = Plan {
            schema: REPORT_SCHEMA,
            session: "a".repeat(64),
            backend: Backend::Kpm,
            mode: Mode::Full,
            payload_sha256: Some("b".repeat(64)),
            probes: Probes::default(),
            kpm_phase: None,
        };
        let mut raw_plan = plan.clone();
        raw_plan.kpm_phase = Some(Phase::Raw);
        let mut target_plan = plan.clone();
        target_plan.kpm_phase = Some(Phase::Protected);
        let raw = PhaseReport {
            plan: raw_plan,
            boot: "12345678-1234-1234-1234-123456789abc".into(),
            setup_ok: true,
            loaded: true,
            counts: HIDE
                .into_iter()
                .chain(KEEP)
                .map(|n| (n.into(), Some(1)))
                .collect(),
            fields: Vec::new(),
        };
        let target = PhaseReport {
            plan: target_plan,
            boot: "12345678-1234-1234-1234-123456789abd".into(),
            counts: HIDE
                .into_iter()
                .map(|n| (n.into(), Some(0)))
                .chain(KEEP.into_iter().map(|n| (n.into(), Some(1))))
                .collect(),
            ..raw.clone()
        };
        (plan, raw, target)
    }
    #[test]
    fn kpm_needs_distinct_complete_immutable_boots_and_positive_baselines() {
        let (plan, raw, target) = fixture();
        assert_eq!(compare_kpm(&plan, &raw, &target).unwrap().rows.len(), 36);
        let mut bad = target.clone();
        bad.boot = raw.boot.clone();
        assert!(compare_kpm(&plan, &raw, &bad).is_err());
        let mut bad = raw.clone();
        bad.counts[0].1 = Some(0);
        assert!(compare_kpm(&plan, &bad, &target).is_err());
        let mut bad = raw.clone();
        bad.counts.push(bad.counts[0].clone());
        assert!(compare_kpm(&plan, &bad, &target).is_err());
        let mut bad = target.clone();
        bad.plan.kpm_phase = Some(Phase::Raw);
        assert!(compare_kpm(&plan, &raw, &bad).is_err());
        let mut bad = target.clone();
        bad.loaded = false;
        assert!(compare_kpm(&plan, &raw, &bad).is_err());
    }
    #[test]
    fn compat_probe_cannot_silently_lose_its_baseline_controls() {
        let (mut plan, _, _) = fixture();
        plan.backend = Backend::Kmod;
        plan.probes.compat.present = true;
        assert!(plan.validate().is_err());
    }

    #[test]
    fn kpm_full_report_rejects_unfiltered_null_query_even_with_clean_fill_and_tail() {
        let (mut plan, mut raw, mut target) = fixture();
        plan.probes.ifconf = Probe {
            present: true,
            required: true,
        };
        raw.plan.probes = plan.probes;
        target.plan.probes = plan.probes;
        let fields = |size, fill, vpn| {
            Some(vec![
                ("IFCONF_SIZE".into(), size),
                ("IFCONF_FILL".into(), fill),
                ("IFCONF_FILL_VPN".into(), vpn),
                ("IFCONF_TAIL".into(), 0),
                ("IFCONF_TAIL_VPN".into(), 0),
            ])
        };
        raw.fields = vec![("ifconf".into(), fields(3, 3, 1))];
        target.fields = vec![("ifconf".into(), fields(2, 2, 0))];
        let report = compare_kpm(&plan, &raw, &target).unwrap();
        assert_eq!(report.rows.len(), 36);
        assert!(
            report
                .rows
                .iter()
                .any(|r| r.vector == "ifconf_probe" && r.verdict == Verdict::Pass)
        );
        target.fields = vec![("ifconf".into(), fields(3, 2, 0))];
        assert!(compare_kpm(&plan, &raw, &target).is_err());
    }
}
