//! Root work the app asks core to do on its behalf (protocol v5).
//!
//! The app runs no root process, so every root read it needs is one of a **fixed** set of
//! probes, and every root write outside the config is one of a fixed set of effects. Nothing
//! here carries a command, a script, or a path chosen by the caller.

use serde::{Deserialize, Serialize};

/// One fixed root probe. Each maps to a script or reader compiled into core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiagnosticProbe {
    /// The dashboard/Hiding root snapshot (framed sections); `packages` adds the
    /// per-user package inventory.
    RootSnapshot { packages: bool },
    /// The bug-report snapshot (network, modules, hook state, inventory).
    DebugSnapshot,
    /// Hook hit counters from the live backend.
    HookCounters,
    /// The kernel ring buffer (read-only; see [`DiagnosticsRequest::clear_dmesg`]).
    Dmesg,
    /// `logcat -d` for the app's own tags.
    LogcatAppTags,
    /// `logcat -d` filtered to the LSPosed / Vector boot lines.
    LogcatLsposedBoot,
    /// `logcat -d` since a timestamp (`MM-DD HH:MM:SS.mmm`), for the debug recorder.
    LogcatSince { since: String },
    /// Boot/kernel partition metadata (paths, sizes, labels).
    KernelPartitions,
    /// Gzipped boot/kernel partition images, written to the export dir.
    KernelImages,
    /// LSPosed's `modules_config.db` (+ wal/shm), copied to the export dir.
    LsposedModulesDb,
    /// Root-side ground truth for the app's checks (`vpnhide_checks`, unhooked).
    GroundTruthChecks,
    /// Hook-inert root observation of VPN presence and `uid`'s routing.
    AppVpnState { uid: u32 },
    /// Installed KPMs as the KernelPatch runtime reports them.
    KpmList,
}

/// A batch of probes, run concurrently by core.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DiagnosticsRequest {
    pub probes: Vec<DiagnosticProbe>,
    /// Destructive: clear the kernel ring buffer **before** the probes run (forensic
    /// capture of a fresh window). Never implied by [`DiagnosticProbe::Dmesg`].
    #[serde(default)]
    pub clear_dmesg: bool,
}

/// One probe's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeOutput {
    pub probe: DiagnosticProbe,
    /// Exit status of the probe (`-1`: could not run or timed out).
    pub exit: i32,
    /// Stdout, inline when small. Empty when it went to [`file`](Self::file).
    pub output: String,
    /// Output larger than the inline cap, written to the export dir (app-readable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Extra artifacts the probe wrote to the export dir (images, db copies).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    /// The output hit the probe's byte cap or its deadline and was cut.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DiagnosticsReport {
    pub outputs: Vec<ProbeOutput>,
    /// Whether `clear_dmesg` ran (and succeeded).
    #[serde(default)]
    pub dmesg_cleared: bool,
}

/// A fixed root write outside the canonical config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RootEffect {
    /// Store the APatch superkey (root-only file) for KPM control.
    SetSuperkey { superkey: String },
    /// Forget the stored superkey.
    ClearSuperkey,
    /// Delete the pre-1.0 per-component config files after an import.
    DeleteLegacyConfig,
    /// Remove every vpnhide state file and dir outside core's own, and the canonical config.
    FullReset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectError {
    /// The request was malformed (e.g. an empty or oversized superkey).
    Invalid(String),
    /// Core is busy with another operation; retry.
    Busy,
    /// The effect ran and failed (detail is safe: no secrets).
    Failed(String),
}
