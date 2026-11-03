//! Local, unprivileged app telemetry readers over the shared wire grammar.
use vpnhide_protocol as protocol;

#[derive(Clone, Copy, uniffi::Record)]
pub struct CoreCounterSample {
    pub current: u64,
    pub baseline: Option<u64>,
}
#[derive(Debug, PartialEq, Eq, uniffi::Enum)]
pub enum CoreCounterChange {
    Unavailable,
    Reset,
    Count { count: u64 },
}
impl From<protocol::telemetry::CounterChange> for CoreCounterChange {
    fn from(value: protocol::telemetry::CounterChange) -> Self {
        match value {
            protocol::telemetry::CounterChange::Unavailable => Self::Unavailable,
            protocol::telemetry::CounterChange::Reset => Self::Reset,
            protocol::telemetry::CounterChange::Count(count) => Self::Count { count },
        }
    }
}
#[derive(Debug, uniffi::Error)]
pub enum CoreCounterError {
    TooManySamples,
}
impl std::fmt::Display for CoreCounterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("counter sample batch exceeds the shared telemetry bound")
    }
}
impl std::error::Error for CoreCounterError {}
#[uniffi::export]
pub fn telemetry_counter_change(
    current: u64,
    baseline: Option<u64>,
    captured: bool,
) -> CoreCounterChange {
    protocol::telemetry::counter_change(current, baseline, captured).into()
}
/// One local FFI call for a capture batch, without a per-row RPC or grant.
#[uniffi::export]
pub fn telemetry_counter_changes(
    samples: Vec<CoreCounterSample>,
    captured: bool,
) -> Result<Vec<CoreCounterChange>, CoreCounterError> {
    if samples.len() > protocol::telemetry::MAX_STATS_CELLS {
        return Err(CoreCounterError::TooManySamples);
    }
    Ok(samples
        .into_iter()
        .map(|sample| telemetry_counter_change(sample.current, sample.baseline, captured))
        .collect())
}
#[uniffi::export]
pub fn protocol_metadata(raw: String) -> std::collections::HashMap<String, String> {
    protocol::telemetry::metadata(&raw).into_iter().collect()
}
fn input(raw: &str) -> Option<&[u8]> {
    (raw.len() <= protocol::telemetry::MAX_TELEMETRY_BYTES).then_some(raw.as_bytes())
}

#[derive(Clone, Copy, uniffi::Enum)]
pub enum CoreProtocolKind {
    Config,
    Stats,
    Status,
}
impl From<CoreProtocolKind> for protocol::Kind {
    fn from(value: CoreProtocolKind) -> Self {
        match value {
            CoreProtocolKind::Config => Self::Config,
            CoreProtocolKind::Stats => Self::Stats,
            CoreProtocolKind::Status => Self::Status,
        }
    }
}
#[derive(uniffi::Record)]
pub struct CoreTelemetryStatusFields {
    pub backend: Option<u32>,
    pub kver: Option<u32>,
    pub hooks: Option<u32>,
    pub error: Option<u32>,
}
#[derive(uniffi::Record)]
pub struct CoreTelemetryStatus {
    pub backend: u32,
    pub kver: u32,
    pub hooks: u32,
    pub error: u32,
}
#[derive(uniffi::Record)]
pub struct CoreTelemetryStat {
    pub uid: u32,
    pub hook_id: u32,
    pub count: u64,
}

#[uniffi::export]
pub fn protocol_peek_kind(raw: String) -> Option<CoreProtocolKind> {
    protocol::peek_kind(input(&raw)?).map(|kind| match kind {
        protocol::Kind::Config => CoreProtocolKind::Config,
        protocol::Kind::Stats => CoreProtocolKind::Stats,
        protocol::Kind::Status => CoreProtocolKind::Status,
    })
}
#[uniffi::export]
pub fn protocol_status_fields(raw: String) -> Option<CoreTelemetryStatusFields> {
    protocol::parse_status_fields(input(&raw)?).map(|s| CoreTelemetryStatusFields {
        backend: s.backend,
        kver: s.kver,
        hooks: s.hooks,
        error: s.error,
    })
}
#[uniffi::export]
pub fn protocol_display_status(raw: String) -> Option<CoreTelemetryStatus> {
    protocol::parse_status_fields(input(&raw)?).map(|s| CoreTelemetryStatus {
        backend: s.backend.unwrap_or_default(),
        kver: s.kver.unwrap_or_default(),
        hooks: s.hooks.unwrap_or_default(),
        error: s.error.unwrap_or_default(),
    })
}
#[uniffi::export]
pub fn protocol_display_stats(raw: String) -> Option<Vec<CoreTelemetryStat>> {
    protocol::telemetry::display_stats(raw.as_bytes()).map(|entries| {
        entries
            .into_iter()
            .map(|s| CoreTelemetryStat {
                uid: s.uid,
                hook_id: s.hook_id,
                count: s.count,
            })
            .collect()
    })
}
#[uniffi::export]
pub fn protocol_extract_block(raw: String, kind: CoreProtocolKind) -> Option<String> {
    protocol::telemetry::extract_block(&raw, kind.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_and_batch_preserve_unsigned_bits_and_the_explicit_baseline_state() {
        let input = vec![
            CoreCounterSample {
                current: u64::MAX,
                baseline: Some(0),
            },
            CoreCounterSample {
                current: 0,
                baseline: Some(u64::MAX),
            },
            CoreCounterSample {
                current: 7,
                baseline: None,
            },
        ];
        assert_eq!(
            telemetry_counter_changes(input.clone(), true).unwrap(),
            vec![
                CoreCounterChange::Count { count: u64::MAX },
                CoreCounterChange::Reset,
                CoreCounterChange::Count { count: 7 },
            ]
        );
        assert_eq!(
            telemetry_counter_changes(input, false).unwrap(),
            vec![
                CoreCounterChange::Unavailable,
                CoreCounterChange::Unavailable,
                CoreCounterChange::Unavailable,
            ]
        );
        assert!(matches!(
            telemetry_counter_changes(
                vec![
                    CoreCounterSample {
                        current: 0,
                        baseline: None
                    };
                    protocol::telemetry::MAX_STATS_CELLS + 1
                ],
                true
            ),
            Err(CoreCounterError::TooManySamples)
        ));
    }
    #[test]
    fn typed_display_does_not_erase_missing_fields_or_unsigned_counter_bits() {
        let missing = protocol_status_fields("vpnhide 1 status\nhooks 0x1\n".into()).unwrap();
        assert_eq!(missing.backend, None);
        assert_eq!(missing.hooks, Some(1));
        let display = protocol_display_status("vpnhide 1 status\nhooks 0x1\n".into()).unwrap();
        assert_eq!(display.backend, 0);
        assert!(protocol_status_fields("vpnhide 2 status\nbackend 0x1\n".into()).is_none());
        let rows =
            protocol_display_stats("vpnhide 1 stats\n0xffffffff 0xff:0xffffffffffffffff\n".into())
                .unwrap();
        assert_eq!(
            (rows[0].uid, rows[0].hook_id, rows[0].count),
            (u32::MAX, 255, u64::MAX)
        );
    }
}
