//! One permitted-file contract for the Core producer, final ZIP and manager
//! staging readback. This is a namespace check, not byte authentication or a
//! claim that optional installer metadata must survive every root manager.
use std::{collections::BTreeSet, io};

use crate::manifest::Manifest;

pub enum CoreLayout<'a> {
    Producer,
    Bundle(&'a Manifest),
    Installed(&'a Manifest),
}

impl CoreLayout<'_> {
    pub fn allowed_files(self) -> io::Result<BTreeSet<String>> {
        let mut files = [
            "module.prop",
            "service.sh",
            "sepolicy.rule",
            "customize.sh",
            "META-INF/com/google/android/update-binary",
            "META-INF/com/google/android/updater-script",
        ]
        .into_iter()
        .map(String::from)
        .collect::<BTreeSet<_>>();
        match self {
            Self::Producer => {
                files.insert("build-provenance.json".into());
                files.insert("bin/arm64/vpnhide-core".into());
                files.insert("bin/arm/vpnhide-core".into());
            }
            Self::Bundle(manifest) | Self::Installed(manifest) => {
                manifest.require_complete()?;
                files.insert("bundle-provenance.json".into());
                files.insert("payloads/manifest.json".into());
                for payload in &manifest.payloads {
                    files.insert(payload.implementation.relative_path()?);
                }
                if matches!(self, Self::Installed(_)) {
                    files.insert("vpnhide-core".into());
                    // This is inert manager staging metadata, not running-image
                    // proof. The device reader additionally requires an empty,
                    // owned regular file; disable/remove/hot-install refuse.
                    files.insert("update".into());
                } else {
                    files.insert("bin/arm64/vpnhide-core".into());
                    files.insert("bin/arm/vpnhide-core".into());
                }
            }
        }
        Ok(files)
    }

    pub fn require_files<'a>(self, files: impl IntoIterator<Item = &'a String>) -> io::Result<()> {
        let allowed = self.allowed_files()?;
        for file in files {
            if !allowed.contains(file) {
                return Err(crate::invalid(format!("unexpected Core member: {file}")));
            }
        }
        Ok(())
    }
}
