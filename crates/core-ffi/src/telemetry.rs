//! Local, unprivileged app telemetry readers over the shared wire grammar.
use vpnhide_protocol as protocol;
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
