//! The canonical config as it crosses the app<->core RPC wire.
//!
//! This is a **clean projection**, not the legacy on-disk canonical JSON. The on-disk
//! format the activator and LSPosed read is polymorphic — `native` is a bool, an array or
//! an object, `java` is a bool or an array — which the app's editable model normalises
//! anyway. The wire is our own boundary, so it carries the normalised, serde-derivable
//! shape: the core owns the durable canonical in whatever format it likes and projects to
//! this for the replica. Mirrors the app's `CanonicalConfig` (StorageConfig.kt) in content.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The whole editable config. `version` tracks the config schema, separate from the IPC
/// [`PROTOCOL_VERSION`](crate::PROTOCOL_VERSION).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalConfig {
    pub version: u32,
    pub debug: bool,
    /// The user's debug-toggle intent, which diverges from `debug` during a capture.
    pub debug_switch: bool,
    pub apps: BTreeMap<String, AppConfig>,
    pub settings: Settings,
}

impl Default for CanonicalConfig {
    fn default() -> Self {
        CanonicalConfig {
            version: 1,
            debug: false,
            debug_switch: false,
            apps: BTreeMap::new(),
            settings: Settings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub remember_superkey: bool,
    pub optional_features: BTreeSet<String>,
    pub auto_hide_vpn_services: bool,
    pub auto_hide_vpn_name: bool,
    pub auto_hide_excluded_packages: BTreeSet<String>,
    pub auto_hidden_packages: BTreeSet<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            remember_superkey: false,
            // Matches the app default (StorageConfig.kt): auto-hiding VPN services is on.
            auto_hide_vpn_services: true,
            auto_hide_vpn_name: false,
            optional_features: BTreeSet::new(),
            auto_hide_excluded_packages: BTreeSet::new(),
            auto_hidden_packages: BTreeSet::new(),
        }
    }
}

/// One targeted app's roles. An app with no role at all is dropped from `apps`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    /// LSPosed (Java) hooks selected wholesale.
    pub java: bool,
    /// A per-hook Java selection; `None` = all when `java` is true.
    pub java_hooks: Option<Vec<String>>,
    pub native: NativeRole,
    pub app_hiding: bool,
    pub ports: bool,
    pub port_policy: Option<PortPolicy>,
    pub hidden: bool,
}

/// The native (kernel/zygisk) role and any per-hook overrides. `None` overrides = all hooks
/// of the active backend family.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRole {
    pub enabled: bool,
    pub kernel: Option<Vec<String>>,
    pub zygisk: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortPolicy {
    pub mode: String,
    pub preset: Option<String>,
    pub rules: Vec<PortRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortRule {
    pub protocol: PortProtocol,
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Both,
    Tcp,
    Udp,
}
