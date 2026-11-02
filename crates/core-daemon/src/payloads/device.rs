//! Read-only capability evidence for bundled selection. File availability and
//! root-manager names are not runtime readiness. Observations do not prove that
//! an eventual load will satisfy ABI/signature checks or hide VPN state in apps.
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use super::Abi;
use super::selection::{Capabilities, Ownership, Runtime};
use crate::native_boot::{KernelFact, KpmFact, NativeEnv, ScanFacts};
use vpnhide_core_rpc::NativeBackend;

fn option(config: &str, name: &str) -> Option<bool> {
    let values: Vec<_> = config
        .lines()
        .filter_map(|line| {
            if line == format!("{name}=y") {
                Some(true)
            } else if line == format!("# {name} is not set") || line == format!("{name}=n") {
                Some(false)
            } else {
                None
            }
        })
        .collect();
    match values.as_slice() {
        [value] => Some(*value),
        _ => None,
    }
}

pub fn module_loading(config: Option<&str>, disabled: Option<&str>) -> Runtime {
    if disabled.map(str::trim) == Some("1") {
        return Runtime::Unsupported;
    }
    let Some(config) = config else {
        return Runtime::Unknown;
    };
    if option(config, "CONFIG_MODULE_SIG_FORCE") == Some(true) {
        return Runtime::Unsupported;
    }
    let required = ["CONFIG_MODULES", "CONFIG_KPROBES", "CONFIG_KRETPROBES"];
    if required
        .iter()
        .any(|name| option(config, name) == Some(false))
    {
        return Runtime::Unsupported;
    }
    if disabled.map(str::trim) != Some("0")
        || required
            .iter()
            .any(|name| option(config, name) != Some(true))
    {
        return Runtime::Unknown;
    }
    Runtime::Ready
}

fn bounded_text(path: &Path, limit: u64) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= limit).then_some(text)
}

fn kernel_config(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() <= 1024 * 1024).then_some(text)
}

/// Positive loader evidence in a root-owned zygote, not installed module names.
/// Unknown is deliberate: unrecognized/masked mappings are not proof of absence.
pub fn zygisk_loader(proc_root: &Path) -> Runtime {
    zygisk_loader_owned_by(proc_root, 0)
}

fn zygisk_loader_owned_by(proc_root: &Path, owner: u32) -> Runtime {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Runtime::Unknown;
    };
    for entry in entries.take(4096).flatten() {
        if !entry
            .file_name()
            .as_encoded_bytes()
            .iter()
            .all(u8::is_ascii_digit)
        {
            continue;
        }
        let path = entry.path();
        if !fs::metadata(&path).is_ok_and(|m| m.uid() == owner && m.is_dir()) {
            continue;
        }
        let Some(cmdline) = bounded_text(&path.join("cmdline"), 256) else {
            continue;
        };
        if !matches!(
            cmdline.split('\0').next(),
            Some("zygote" | "zygote64" | "zygote32")
        ) {
            continue;
        }
        let Some(maps) = bounded_text(&path.join("maps"), 1024 * 1024) else {
            continue;
        };
        // NeoZygisk's named library is positive loader evidence only. Other
        // providers/anonymized mappings remain Unknown, never guessed from names
        // of installed manager modules. See upstream NeoZygisk release notes.
        if maps.lines().any(|line| {
            line.split_whitespace()
                .any(|part| part.ends_with("/libzygisk.so"))
        }) {
            return Runtime::Ready;
        }
    }
    Runtime::Unknown
}

pub fn observe(env: &NativeEnv, facts: &ScanFacts) -> Capabilities {
    let release = vpnhide_activator::sys::uname()
        .map(|u| u.release)
        .unwrap_or_default();
    let config = kernel_config(Path::new(crate::native_boot::CONFIG_GZ));
    let disabled = bounded_text(Path::new("/proc/sys/kernel/modules_disabled"), 16);
    let kpm = match &facts.kpm {
        KpmFact::Absent | KpmFact::Live => match vpnhide_activator::running_kernel_kpm_support() {
            Ok((_, true)) => Runtime::Ready,
            Ok((_, false)) => Runtime::Unsupported,
            Err(_) => Runtime::Unknown,
        },
        KpmFact::NoRuntime | KpmFact::NotManaged => Runtime::Absent,
        KpmFact::AuthFailed(_) => Runtime::AuthenticationRequired,
        KpmFact::Incompatible { .. } => Runtime::Unsupported,
        KpmFact::Broken(_) => Runtime::Unknown,
    };
    let live = facts.live();
    let ownership = if !facts.installed.is_empty()
        || matches!(
            facts.kernel,
            KernelFact::Foreign(_) | KernelFact::Unreadable(_)
        )
        || matches!(
            facts.kpm,
            KpmFact::AuthFailed(_) | KpmFact::Broken(_) | KpmFact::Incompatible { .. }
        )
        || env
            .modules
            .as_deref()
            .is_some_and(|path| !crate::native_boot::obsolete_modules(path).is_empty())
    {
        Ownership::Unknown
    } else {
        match live.as_slice() {
            [] => Ownership::Clear,
            [backend] => Ownership::Resident(*backend),
            _ => Ownership::Unknown,
        }
    };
    Capabilities {
        // Bundled kernel payloads are arm64 only. A host build is not Android ARM64.
        abi: if cfg!(target_arch = "aarch64") {
            Abi::Arm64
        } else {
            Abi::Arm32
        },
        kernel_release: release,
        builtin: if facts.kernel == KernelFact::Builtin {
            Runtime::Ready
        } else {
            Runtime::Absent
        },
        module_loading: module_loading(config.as_deref(), disabled.as_deref()),
        kpm,
        zygisk: zygisk_loader(Path::new("/proc")),
        // Both ABIs ship. Requiring both on arm64 is conservative, including mixed zygotes.
        needs_arm32_zygisk: true,
        ownership,
    }
}

/// A resident backend must still pass its own complete projection and coverage
/// checks; residency alone never overrides rejection or permits a second backend.
pub fn resident_eligible(
    assessment: &super::selection::Assessment,
    backend: NativeBackend,
) -> bool {
    assessment
        .candidates
        .iter()
        .any(|candidate| candidate.backend == backend && candidate.eligibility.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loader_evidence_requires_owned_zygote_mapping_not_manager_files() {
        let root = tempfile::tempdir().unwrap();
        let process = root.path().join("123");
        fs::create_dir(&process).unwrap();
        let owner = fs::metadata(&process).unwrap().uid();
        fs::write(process.join("cmdline"), b"unrelated\0").unwrap();
        fs::write(
            process.join("maps"),
            "a-b r-xp 0000 00:00 0 /data/adb/neozygisk/lib64/libzygisk.so\n",
        )
        .unwrap();
        assert_eq!(zygisk_loader_owned_by(root.path(), owner), Runtime::Unknown);
        fs::write(process.join("cmdline"), b"zygote64\0").unwrap();
        assert_eq!(zygisk_loader_owned_by(root.path(), owner), Runtime::Ready);
        assert_eq!(
            zygisk_loader_owned_by(root.path(), owner.wrapping_add(1)),
            Runtime::Unknown
        );
        fs::write(process.join("maps"), "anonymous provider mapping\n").unwrap();
        assert_eq!(zygisk_loader_owned_by(root.path(), owner), Runtime::Unknown);
    }

    #[test]
    fn required_kernel_configuration_is_not_inferred_from_missing_evidence() {
        let config = "CONFIG_MODULES=y\nCONFIG_KPROBES=y\nCONFIG_KRETPROBES=y\n";
        assert_eq!(module_loading(Some(config), Some("0\n")), Runtime::Ready);
        assert_eq!(module_loading(None, Some("0")), Runtime::Unknown);
        assert_eq!(module_loading(Some(config), None), Runtime::Unknown);
        assert_eq!(
            module_loading(Some(config), Some("1")),
            Runtime::Unsupported
        );
        assert_eq!(
            module_loading(
                Some(&format!("{config}CONFIG_MODULE_SIG_FORCE=y\n")),
                Some("0")
            ),
            Runtime::Unsupported
        );
        assert_eq!(
            module_loading(
                Some(&config.replace("CONFIG_KRETPROBES=y", "# CONFIG_KRETPROBES is not set")),
                Some("0")
            ),
            Runtime::Unsupported
        );
        assert_eq!(
            module_loading(Some(&format!("{config}CONFIG_KPROBES=y\n")), Some("0")),
            Runtime::Unknown
        );
    }
}
