//! Shared bounded native-endian netlink framing. No sockets or fixture effects.
use vpnhide_diagnostic_model::ProbeError;

pub fn u16_at(bytes: &[u8], at: usize) -> Result<u16, ProbeError> {
    Ok(u16::from_ne_bytes(
        bytes
            .get(at..at.checked_add(2).ok_or(ProbeError::Malformed)?)
            .ok_or(ProbeError::Malformed)?
            .try_into()
            .map_err(|_| ProbeError::Malformed)?,
    ))
}
pub fn u32_at(bytes: &[u8], at: usize) -> Result<u32, ProbeError> {
    Ok(u32::from_ne_bytes(
        bytes
            .get(at..at.checked_add(4).ok_or(ProbeError::Malformed)?)
            .ok_or(ProbeError::Malformed)?
            .try_into()
            .map_err(|_| ProbeError::Malformed)?,
    ))
}
pub struct Message<'a> {
    pub kind: u16,
    pub flags: u16,
    pub sequence: u32,
    pub body: &'a [u8],
}
// Decode one frame at a time so read-only consumers can retain a valid prefix
// when a later frame is corrupt. Full-array callers (notably mutation ACKs)
// still receive Err for any malformed suffix.
struct Frames<'a> {
    bytes: &'a [u8],
    at: usize,
    failed: bool,
}
impl<'a> Frames<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, ProbeError> {
        if bytes.is_empty() || bytes.len() > 1024 * 1024 {
            return Err(ProbeError::Malformed);
        }
        Ok(Self {
            bytes,
            at: 0,
            failed: false,
        })
    }
    fn next(&mut self) -> Option<Result<Message<'a>, ProbeError>> {
        if self.failed || self.at == self.bytes.len() {
            return None;
        }
        let at = self.at;
        let result = (|| {
            let length = u32_at(self.bytes, at)? as usize;
            if length < 16 || length > self.bytes.len() - at {
                return Err(ProbeError::Malformed);
            }
            let aligned = (length + 3) & !3;
            if aligned > self.bytes.len() - at {
                return Err(ProbeError::Malformed);
            }
            let message = Message {
                kind: u16_at(self.bytes, at + 4)?,
                flags: u16_at(self.bytes, at + 6)?,
                sequence: u32_at(self.bytes, at + 8)?,
                body: &self.bytes[at + 16..at + length],
            };
            self.at += aligned;
            Ok(message)
        })();
        if result.is_err() {
            self.failed = true;
        }
        Some(result)
    }
}
pub fn messages(bytes: &[u8]) -> Result<Vec<Message<'_>>, ProbeError> {
    let mut frames = Frames::new(bytes)?;
    let mut result = Vec::new();
    while let Some(frame) = frames.next() {
        result.push(frame?);
    }
    Ok(result)
}

pub struct Attribute<'a> {
    pub kind: u16,
    pub value: &'a [u8],
}
pub fn attributes(bytes: &[u8]) -> Result<Vec<Attribute<'_>>, ProbeError> {
    let mut result = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let length = u16_at(bytes, at)? as usize;
        if length < 4 || length > bytes.len() - at {
            return Err(ProbeError::Malformed);
        }
        let aligned = (length + 3) & !3;
        if aligned > bytes.len() - at {
            return Err(ProbeError::Malformed);
        }
        result.push(Attribute {
            kind: u16_at(bytes, at + 2)?,
            value: &bytes[at + 4..at + length],
        });
        at += aligned;
    }
    Ok(result)
}
pub fn interface_name(value: &[u8]) -> Result<&str, ProbeError> {
    let Some((&0, name)) = value.split_last() else {
        return Err(ProbeError::Malformed);
    };
    if name.is_empty()
        || name.len() > 15
        || !name.is_ascii()
        || name.iter().any(|b| b.is_ascii_control())
    {
        return Err(ProbeError::Malformed);
    }
    std::str::from_utf8(name).map_err(|_| ProbeError::Malformed)
}

pub struct Dump {
    sequence: u32,
    data_kind: u16,
    done: bool,
    failed: bool,
}
impl Dump {
    pub fn new(sequence: u32, data_kind: u16) -> Self {
        Self {
            sequence,
            data_kind,
            done: false,
            failed: false,
        }
    }
    pub fn consume(
        &mut self,
        bytes: &[u8],
        mut data: impl FnMut(&[u8]) -> Result<(), ProbeError>,
    ) -> Result<bool, ProbeError> {
        if self.done || self.failed {
            return Err(ProbeError::Malformed);
        }
        let result = self.consume_inner(bytes, &mut data);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn consume_inner(
        &mut self,
        bytes: &[u8],
        data: &mut impl FnMut(&[u8]) -> Result<(), ProbeError>,
    ) -> Result<bool, ProbeError> {
        let mut frames = Frames::new(bytes)?;
        while let Some(frame) = frames.next() {
            let frame = frame?;
            if frame.sequence != self.sequence || frame.flags & 0x10 != 0 {
                return Err(ProbeError::Malformed);
            }
            match frame.kind {
                3 => {
                    if frames.at != bytes.len()
                        || !matches!(frame.body.len(), 0 | 4)
                        || (frame.body.len() == 4 && u32_at(frame.body, 0)? != 0)
                    {
                        return Err(ProbeError::Malformed);
                    }
                    self.done = true;
                }
                2 => {
                    let error = i32::from_ne_bytes(u32_at(frame.body, 0)?.to_ne_bytes())
                        .checked_neg()
                        .filter(|e| *e > 0)
                        .ok_or(ProbeError::Malformed)?;
                    return Err(ProbeError::Errno(error));
                }
                kind if kind == self.data_kind => data(frame.body)?,
                _ => return Err(ProbeError::Malformed),
            }
        }
        Ok(self.done)
    }
}

/// Mutation ACKs must echo the exact request header. A timeout or
/// malformed ACK is never success; the caller still owns readback and cleanup.
pub fn acknowledgement(bytes: &[u8], request: &[u8]) -> Result<(), ProbeError> {
    if request.len() < 16 || u32_at(request, 0)? as usize != request.len() {
        return Err(ProbeError::Malformed);
    }
    let frames = messages(bytes)?;
    if frames.len() != 1 {
        return Err(ProbeError::Malformed);
    }
    let frame = &frames[0];
    if frame.kind != 2
        || frame.sequence != u32_at(request, 8)?
        || frame.flags & 0x10 != 0
        || frame.body.len() < 20
        || frame.body[4..20] != request[..16]
    {
        return Err(ProbeError::Malformed);
    }
    let error = i32::from_ne_bytes(u32_at(frame.body, 0)?.to_ne_bytes());
    if error == 0 {
        Ok(())
    } else {
        Err(ProbeError::Errno(
            error
                .checked_neg()
                .filter(|e| *e > 0)
                .ok_or(ProbeError::Malformed)?,
        ))
    }
}
