//! Bounded evidence from the exact extracted ARM64 kernel image. A successful
//! external config-dump exit status is never config evidence.
use crate::{ProvisionError, sha256};
use flate2::bufread::GzDecoder;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Read;

const MAX_IMAGE: usize = 256 * 1024 * 1024;
const MAX_CONFIG: u64 = 2 * 1024 * 1024;
const START: &[u8] = b"IKCFG_ST";
const END: &[u8] = b"IKCFG_ED";
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KernelEvidence {
    pub(crate) image_sha256: String,
    pub(crate) image_bytes: u64,
    pub(crate) config_sha256: String,
    pub(crate) values: BTreeMap<String, String>,
}
impl KernelEvidence {
    pub fn image_sha256(&self) -> &str {
        &self.image_sha256
    }
    pub fn config_sha256(&self) -> &str {
        &self.config_sha256
    }
    pub fn image_bytes(&self) -> u64 {
        self.image_bytes
    }
    pub fn enabled(&self, name: &str) -> Option<bool> {
        match self.values.get(name).map(String::as_str) {
            Some("y") => Some(true),
            Some("n") => Some(false),
            _ => None,
        }
    }
}
fn parse_config(raw: &[u8]) -> Result<BTreeMap<String, String>, ProvisionError> {
    let text = std::str::from_utf8(raw).map_err(|_| ProvisionError::KernelConfigUnknown)?;
    if text.contains('\0') || !text.ends_with('\n') {
        return Err(ProvisionError::KernelConfigUnknown);
    }
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let pair = if let Some(unset) = line
            .strip_prefix("# ")
            .and_then(|v| v.strip_suffix(" is not set"))
        {
            Some((unset, "n"))
        } else if line.starts_with("CONFIG_") {
            line.split_once('=')
        } else {
            None
        };
        let Some((name, value)) = pair else {
            continue;
        };
        if !name.starts_with("CONFIG_")
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || value.is_empty()
        {
            return Err(ProvisionError::KernelConfigUnknown);
        }
        if values.insert(name.into(), value.into()).is_some() {
            return Err(ProvisionError::KernelConfigUnknown);
        }
    }
    if values.is_empty() {
        return Err(ProvisionError::KernelConfigUnknown);
    }
    Ok(values)
}
pub fn inspect_kernel(image: &[u8]) -> Result<KernelEvidence, ProvisionError> {
    if image.len() < 64 || image.len() > MAX_IMAGE || &image[56..60] != b"ARM\x64" {
        return Err(ProvisionError::KernelImageInvalid);
    }
    let image_size = u64::from_le_bytes(image[16..24].try_into().expect("header range"));
    // image_size includes runtime BSS; it can exceed the on-disk Image length.
    if !(64..=1024 * 1024 * 1024).contains(&image_size) {
        return Err(ProvisionError::KernelImageInvalid);
    }
    let mut found = None;
    let mut markers = 0;
    for (index, window) in image.windows(START.len()).enumerate() {
        if window != START {
            continue;
        }
        markers += 1;
        if markers > 16 {
            return Err(ProvisionError::KernelConfigUnknown);
        }
        let start = index + START.len();
        let input = &image[start..];
        if !input.starts_with(&[0x1f, 0x8b, 8]) {
            continue;
        }
        let mut decoder = GzDecoder::new(input);
        let mut raw = Vec::new();
        (&mut decoder)
            .take(MAX_CONFIG + 1)
            .read_to_end(&mut raw)
            .map_err(|_| ProvisionError::KernelConfigUnknown)?;
        if raw.len() as u64 > MAX_CONFIG {
            return Err(ProvisionError::KernelConfigUnknown);
        }
        // bufread GzDecoder consumes exactly one member including its CRC/trailer.
        let consumed = input.len() - decoder.into_inner().len();
        if !input
            .get(consumed..)
            .is_some_and(|tail| tail.starts_with(END))
            || found.is_some()
        {
            return Err(ProvisionError::KernelConfigUnknown);
        }
        found = Some((sha256(&raw), parse_config(&raw)?));
    }
    let (config_sha256, values) = found.ok_or(ProvisionError::KernelConfigUnknown)?;
    Ok(KernelEvidence {
        image_sha256: sha256(image),
        image_bytes: image.len() as u64,
        config_sha256,
        values,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;
    pub fn image(config: &str) -> Vec<u8> {
        let mut image = vec![0; 64];
        image[56..60].copy_from_slice(b"ARM\x64");
        image.extend_from_slice(START);
        let mut compressed = GzEncoder::new(Vec::new(), Compression::default());
        compressed.write_all(config.as_bytes()).unwrap();
        image.extend_from_slice(&compressed.finish().unwrap());
        image.extend_from_slice(END);
        let size = image.len() as u64;
        image[16..24].copy_from_slice(&size.to_le_bytes());
        image
    }
    #[test]
    fn arm64_runtime_memory_size_may_exceed_file_length() {
        let mut bytes =
            image("CONFIG_KALLSYMS=y\nCONFIG_SND_SOC_PCM512x_I2C=y\nCONFIG_FONT_8x16=y\n");
        bytes[16..24].copy_from_slice(&26_476_544u64.to_le_bytes());
        assert!(inspect_kernel(&bytes).is_ok());
    }
    #[test]
    fn evidence_is_hash_bound_and_disabled_all_is_not_missing_kallsyms() {
        let bytes = image("CONFIG_KALLSYMS=y\n# CONFIG_KALLSYMS_ALL is not set\n");
        let facts = inspect_kernel(&bytes).unwrap();
        assert_eq!(facts.enabled("CONFIG_KALLSYMS"), Some(true));
        assert_eq!(facts.enabled("CONFIG_KALLSYMS_ALL"), Some(false));
        assert_eq!(facts.image_sha256, sha256(&bytes));
        assert_eq!(facts.enabled("CONFIG_KPM"), None);
    }
    #[test]
    fn missing_duplicate_truncated_bad_crc_and_oversized_configs_never_become_evidence() {
        let valid = image("CONFIG_KALLSYMS=y\n");
        let mut cases = vec![
            image(""),
            image("CONFIG_KALLSYMS=y\nCONFIG_KALLSYMS=n\n"),
            image("CONFIG_KALLSYMS=y"),
        ];
        let mut crc = valid.clone();
        let position = crc.len() - END.len() - 8;
        crc[position] ^= 1;
        cases.push(crc);
        let mut truncated = valid.clone();
        truncated.truncate(truncated.len() - 1);
        cases.push(truncated);
        let mut duplicate = valid.clone();
        duplicate.extend_from_slice(&valid[64..]);
        cases.push(duplicate);
        cases.push(image(&format!(
            "# {}\nCONFIG_KALLSYMS=y\n",
            "x".repeat(MAX_CONFIG as usize)
        )));
        for bytes in cases {
            assert!(inspect_kernel(&bytes).is_err());
        }
    }
}
