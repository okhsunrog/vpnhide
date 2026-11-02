//! Portable maintenance client contracts. No root effects, transport or daemon.
use serde::{Deserialize, Serialize};
pub const RESET_SCHEMA: u32 = 1;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetStatus {
    Allocated,
    Accepted,
    Running,
    Confirmed,
    FailedKnown,
    Unknown,
    NotFound,
    Rejected,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetReply {
    pub schema: u32,
    pub boot_id: Option<String>,
    pub epoch: Option<u64>,
    pub operation: Option<u64>,
    pub status: ResetStatus,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceIdentity {
    pub serial: String,
    pub boot_id: String,
}
impl DeviceIdentity {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.serial.is_empty()
            || self.serial.len() > 128
            || !self
                .serial
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
        {
            return Err("invalid explicit ADB serial");
        }
        if self.boot_id.len() != 36
            || !self.boot_id.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            })
        {
            return Err("invalid device boot identity");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetPhase {
    Prepared,
    Allocated,
    SubmitIntent,
    Waiting,
    Confirmed,
    FailedKnown,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetSession {
    pub schema: u32,
    pub device: DeviceIdentity,
    pub operation: Option<u64>,
    pub phase: ResetPhase,
}
impl ResetSession {
    pub fn new(device: DeviceIdentity) -> Result<Self, &'static str> {
        device.validate()?;
        Ok(Self {
            schema: RESET_SCHEMA,
            device,
            operation: None,
            phase: ResetPhase::Prepared,
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.device.validate()?;
        if self.schema != RESET_SCHEMA
            || self.operation == Some(0)
            || ((self.phase == ResetPhase::Prepared) != self.operation.is_none())
        {
            return Err("invalid reset session");
        }
        Ok(())
    }
    pub fn allocated(&mut self, reply: &ResetReply) -> Result<(), &'static str> {
        if self.phase != ResetPhase::Prepared
            || reply.schema != RESET_SCHEMA
            || reply.status != ResetStatus::Allocated
            || !reply.epoch.is_some_and(|v| v > 0)
            || !reply.operation.is_some_and(|v| v > 0)
            || reply.boot_id.as_deref() != Some(self.device.boot_id.as_str())
        {
            return Err("allocation was not confirmed");
        }
        self.operation = reply.operation;
        self.phase = ResetPhase::Allocated;
        Ok(())
    }
    /// Persist this transition BEFORE executing Submit. Resuming SubmitIntent
    /// may only query its identity, even if the client crashed before sending.
    pub fn begin_submit(&mut self, device: &DeviceIdentity) -> Result<u64, &'static str> {
        self.validate()?;
        if &self.device != device || self.phase != ResetPhase::Allocated {
            return Err(
                "reset submission requires the exact prepared device/boot and unused intent",
            );
        }
        self.phase = ResetPhase::SubmitIntent;
        self.operation.ok_or("operation missing")
    }
    pub fn observe(&mut self, reply: &ResetReply) -> Result<(), &'static str> {
        self.validate()?;
        if !matches!(
            self.phase,
            ResetPhase::SubmitIntent | ResetPhase::Waiting | ResetPhase::Unknown
        ) || reply.schema != RESET_SCHEMA
            || reply.operation != self.operation
            || reply.boot_id.as_deref() != Some(self.device.boot_id.as_str())
            || (reply.status != ResetStatus::Unknown && !reply.epoch.is_some_and(|v| v > 0))
        {
            return Err("receipt does not identify the submitted reset");
        }
        self.phase = match reply.status {
            ResetStatus::Confirmed => ResetPhase::Confirmed,
            ResetStatus::FailedKnown | ResetStatus::Rejected => ResetPhase::FailedKnown,
            ResetStatus::Accepted | ResetStatus::Running => ResetPhase::Waiting,
            ResetStatus::Unknown | ResetStatus::NotFound => ResetPhase::Unknown,
            ResetStatus::Allocated => return Err("allocation is not reset completion"),
        };
        Ok(())
    }
    pub fn lost_reply(&mut self) -> Result<(), &'static str> {
        if !matches!(
            self.phase,
            ResetPhase::SubmitIntent | ResetPhase::Waiting | ResetPhase::Unknown
        ) {
            return Err("no submitted reset to reconcile");
        }
        self.phase = ResetPhase::Unknown;
        Ok(())
    }
    pub fn recheck_confirmation(&mut self) -> Result<(), &'static str> {
        if self.phase != ResetPhase::Confirmed {
            return Err("no confirmation to recheck");
        }
        self.phase = ResetPhase::Waiting;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn device() -> DeviceIdentity {
        DeviceIdentity {
            serial: "test-device".into(),
            boot_id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
        }
    }
    fn reply(status: ResetStatus) -> ResetReply {
        ResetReply {
            schema: RESET_SCHEMA,
            boot_id: Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into()),
            epoch: Some(9),
            operation: Some(17),
            status,
        }
    }
    #[test]
    fn lost_dispatch_or_reply_queries_same_id_and_never_authorizes_second_submit() {
        let d = device();
        let mut s = ResetSession::new(d.clone()).unwrap();
        s.allocated(&reply(ResetStatus::Allocated)).unwrap();
        assert_eq!(s.begin_submit(&d).unwrap(), 17);
        s.lost_reply().unwrap();
        assert!(s.begin_submit(&d).is_err());
        s.observe(&reply(ResetStatus::NotFound)).unwrap();
        assert_eq!(s.phase, ResetPhase::Unknown);
        assert!(s.begin_submit(&d).is_err());
        s.observe(&reply(ResetStatus::Confirmed)).unwrap();
        assert_eq!(s.phase, ResetPhase::Confirmed);
        assert!(s.begin_submit(&d).is_err());
    }
    #[test]
    fn target_boot_change_unissued_id_and_foreign_receipt_refuse() {
        let d = device();
        let mut s = ResetSession::new(d.clone()).unwrap();
        assert!(s.begin_submit(&d).is_err());
        s.allocated(&reply(ResetStatus::Allocated)).unwrap();
        let mut other = d.clone();
        other.boot_id = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into();
        assert!(s.begin_submit(&other).is_err());
        assert_eq!(s.phase, ResetPhase::Allocated);
        s.begin_submit(&d).unwrap();
        let mut foreign = reply(ResetStatus::Confirmed);
        foreign.operation = Some(18);
        assert!(s.observe(&foreign).is_err());
        assert_eq!(s.phase, ResetPhase::SubmitIntent);
    }
    #[test]
    fn accepted_running_and_known_failure_are_distinct_from_confirmation() {
        let d = device();
        let mut s = ResetSession::new(d.clone()).unwrap();
        s.allocated(&reply(ResetStatus::Allocated)).unwrap();
        s.begin_submit(&d).unwrap();
        s.observe(&reply(ResetStatus::Accepted)).unwrap();
        assert_eq!(s.phase, ResetPhase::Waiting);
        s.observe(&reply(ResetStatus::Running)).unwrap();
        assert_eq!(s.phase, ResetPhase::Waiting);
        s.observe(&reply(ResetStatus::FailedKnown)).unwrap();
        assert_eq!(s.phase, ResetPhase::FailedKnown);
        assert!(s.begin_submit(&d).is_err());
    }
    #[test]
    fn malformed_device_and_session_never_become_submission_authority() {
        let mut d = device();
        d.serial = "serial;echo injected".into();
        assert!(ResetSession::new(d).is_err());
        let mut s = ResetSession::new(device()).unwrap();
        s.phase = ResetPhase::Allocated;
        assert!(s.validate().is_err());
        s.operation = Some(0);
        assert!(s.validate().is_err());
    }
}
