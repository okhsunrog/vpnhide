use std::io::{self, Read};
use std::os::unix::net::UnixStream;

use serde::{Deserialize, Serialize};

/// The only command output retained in a receipt: validated scalar capacity evidence.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCapacity {
    pub total: u32,
    pub cap: u32,
    pub dropped: u32,
}

impl NativeCapacity {
    pub fn valid(&self) -> bool {
        self.cap > 0
            && self.total <= i32::MAX as u32
            && self.total > self.cap
            && self.dropped == self.total - self.cap
    }
}

pub struct Output {
    reader: UnixStream,
    line: Vec<u8>,
    oversized: bool,
    pub warning: Option<NativeCapacity>,
}

impl Output {
    pub fn new(reader: UnixStream) -> io::Result<Self> {
        reader.set_nonblocking(true)?;
        Ok(Self {
            reader,
            line: Vec::with_capacity(256),
            oversized: false,
            warning: None,
        })
    }

    /// Bound each pass so continuous output cannot starve waitpid or the command deadline.
    pub fn drain(&mut self) -> io::Result<bool> {
        let mut buffer = [0; 4096];
        for _ in 0..16 {
            match self.reader.read(&mut buffer) {
                Ok(0) => {
                    self.finish_line();
                    return Ok(true);
                }
                Ok(count) => {
                    for &byte in &buffer[..count] {
                        if byte == b'\n' {
                            self.finish_line();
                        } else if self.line.len() < 256 {
                            self.line.push(byte);
                        } else {
                            self.oversized = true;
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(false)
    }

    fn finish_line(&mut self) {
        if !self.oversized
            && let Some(warning) = parse_capacity(&self.line)
        {
            self.warning = Some(warning);
        }
        self.line.clear();
        self.oversized = false;
    }
}

fn parse_capacity(line: &[u8]) -> Option<NativeCapacity> {
    let line = std::str::from_utf8(line).ok()?;
    let rest = line.strip_prefix("vpnhide-warning native_target_cap total=")?;
    let (total, rest) = rest.split_once(" cap=")?;
    let (cap, dropped) = rest.split_once(" dropped=")?;
    let count = |value: &str| -> Option<u32> {
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        value.parse().ok()
    };
    let warning = NativeCapacity {
        total: count(total)?,
        cap: count(cap)?,
        dropped: count(dropped)?,
    };
    warning.valid().then_some(warning)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn only_exact_valid_scalar_warnings_survive_output() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let mut output = Output::new(reader).unwrap();
        writer
            .write_all(b"secret text\nvpnhide-warning native_target_cap total=10 cap=8 dropped=2\n")
            .unwrap();
        assert!(!output.drain().unwrap());
        for line in [
            "vpnhide-warning native_target_cap total=20 cap=0 dropped=20",
            "vpnhide-warning native_target_cap total=20 cap=8 dropped=1",
            "vpnhide-warning native_target_cap total=20 cap=8 dropped=12 secret",
            "vpnhide-warning native_target_cap total=2147483648 cap=8 dropped=2147483640",
        ] {
            assert!(parse_capacity(line.as_bytes()).is_none());
        }
        let long =
            "x".repeat(1024) + "vpnhide-warning native_target_cap total=20 cap=8 dropped=12\n";
        writer.write_all(long.as_bytes()).unwrap();
        drop(writer);
        assert!(output.drain().unwrap());
        let warning = output.warning.unwrap();
        assert_eq!(warning.total, 10);
        assert_eq!(warning.cap, 8);
        assert_eq!(warning.dropped, 2);
        assert_eq!(output.line.capacity(), 256);
    }
}
