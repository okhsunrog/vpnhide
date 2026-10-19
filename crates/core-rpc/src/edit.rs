//! Typed field intents (core-2.0 §5) — the edit vocabulary a client submits, mirroring the
//! app's `CanonicalEdit` (CanonicalEditData.kt). A client sends the change it wants, never
//! an authoritative replacement of a screen snapshot; the core prepares each against fresh
//! state and can reject a stale one as a conflict.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, CanonicalConfig};

/// One typed edit against the canonical config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigEdit {
    /// Flip a single boolean setting.
    Toggle { field: ConfigToggle, enabled: bool },
    /// Replace a whole string-set setting (e.g. the auto-hide exclusions).
    SettingSet {
        field: ConfigSetField,
        values: BTreeSet<String>,
    },
    /// Set one field of one app's config; an app left with no role is dropped.
    App {
        pkg: String,
        field: ConfigAppField,
        value: AppConfig,
    },
    /// Drop an app entirely.
    RemoveApp { pkg: String },
    /// Replace the whole config — a broad write set (import / reset).
    Replace { config: CanonicalConfig },
    /// Re-apply native activation without changing the durable config (§5 repair).
    /// Persisted intent and runtime application stay separate facts.
    RepairActivation,
}

/// A single boolean setting addressable by [`ConfigEdit::Toggle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigToggle {
    Filesystem,
    Debug,
    DebugSwitch,
    RememberSuperkey,
    AutoHideServices,
    AutoHideName,
}

/// A string-set setting addressable by [`ConfigEdit::SettingSet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigSetField {
    OptionalFeatures,
    AutoHideExcludedPackages,
    AutoHiddenPackages,
}

/// A per-app field addressable by [`ConfigEdit::App`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigAppField {
    Java,
    Native,
    AppHiding,
    Ports,
    Hidden,
}
