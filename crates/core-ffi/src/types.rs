//! Typed app boundary. Domain collections are normalized here, independently of the JSON RPC codec.
use std::collections::HashMap;
use vpnhide_core_rpc as rpc;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreDebugLeaseOwner {
    Forensic,
    Logcat,
}
impl From<rpc::DebugLeaseOwner> for CoreDebugLeaseOwner {
    fn from(value: rpc::DebugLeaseOwner) -> Self {
        match value {
            rpc::DebugLeaseOwner::Forensic => Self::Forensic,
            rpc::DebugLeaseOwner::Logcat => Self::Logcat,
        }
    }
}
impl From<CoreDebugLeaseOwner> for rpc::DebugLeaseOwner {
    fn from(value: CoreDebugLeaseOwner) -> Self {
        match value {
            CoreDebugLeaseOwner::Forensic => Self::Forensic,
            CoreDebugLeaseOwner::Logcat => Self::Logcat,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreDebugLease {
    pub id: u64,
    pub owner: CoreDebugLeaseOwner,
    pub expires_at_ms: u64,
}
impl From<rpc::DebugLease> for CoreDebugLease {
    fn from(value: rpc::DebugLease) -> Self {
        Self {
            id: value.id.0,
            owner: value.owner.into(),
            expires_at_ms: value.expires_at_ms,
        }
    }
}
impl From<CoreDebugLease> for rpc::DebugLease {
    fn from(value: CoreDebugLease) -> Self {
        Self {
            id: rpc::DebugLeaseId(value.id),
            owner: value.owner.into(),
            expires_at_ms: value.expires_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreApplyEvidence {
    pub phase: CoreConfigPhase,
    pub desired_revision: u64,
    pub op: u64,
    pub outcome: CorePhaseOutcome,
}
impl From<rpc::ApplyEvidence> for CoreApplyEvidence {
    fn from(value: rpc::ApplyEvidence) -> Self {
        Self {
            phase: value.phase.into(),
            desired_revision: value.desired_revision,
            op: value.op.0,
            outcome: value.outcome.into(),
        }
    }
}
impl From<CoreApplyEvidence> for rpc::ApplyEvidence {
    fn from(value: CoreApplyEvidence) -> Self {
        Self {
            phase: value.phase.into(),
            desired_revision: value.desired_revision,
            op: rpc::OperationId(value.op),
            outcome: value.outcome.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreCanonicalConfig {
    pub version: u32,
    pub debug: bool,
    pub debug_switch: bool,
    pub apps: HashMap<String, CoreAppConfig>,
    pub settings: CoreSettings,
}
impl From<rpc::CanonicalConfig> for CoreCanonicalConfig {
    fn from(value: rpc::CanonicalConfig) -> Self {
        Self {
            version: value.version,
            debug: value.debug,
            debug_switch: value.debug_switch,
            apps: value
                .apps
                .into_iter()
                .map(|(key, value)| (key, value.into()))
                .collect(),
            settings: value.settings.into(),
        }
    }
}
impl From<CoreCanonicalConfig> for rpc::CanonicalConfig {
    fn from(value: CoreCanonicalConfig) -> Self {
        Self {
            version: value.version,
            debug: value.debug,
            debug_switch: value.debug_switch,
            apps: value
                .apps
                .into_iter()
                .map(|(key, value)| (key, value.into()))
                .collect(),
            settings: value.settings.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreSettings {
    pub remember_superkey: bool,
    pub optional_features: Vec<String>,
    pub auto_hide_vpn_services: bool,
    pub auto_hide_vpn_name: bool,
    pub auto_hide_excluded_packages: Vec<String>,
    pub auto_hidden_packages: Vec<String>,
}
impl From<rpc::Settings> for CoreSettings {
    fn from(value: rpc::Settings) -> Self {
        Self {
            remember_superkey: value.remember_superkey,
            optional_features: value.optional_features.into_iter().collect(),
            auto_hide_vpn_services: value.auto_hide_vpn_services,
            auto_hide_vpn_name: value.auto_hide_vpn_name,
            auto_hide_excluded_packages: value.auto_hide_excluded_packages.into_iter().collect(),
            auto_hidden_packages: value.auto_hidden_packages.into_iter().collect(),
        }
    }
}
impl From<CoreSettings> for rpc::Settings {
    fn from(value: CoreSettings) -> Self {
        Self {
            remember_superkey: value.remember_superkey,
            optional_features: value.optional_features.into_iter().collect(),
            auto_hide_vpn_services: value.auto_hide_vpn_services,
            auto_hide_vpn_name: value.auto_hide_vpn_name,
            auto_hide_excluded_packages: value.auto_hide_excluded_packages.into_iter().collect(),
            auto_hidden_packages: value.auto_hidden_packages.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreAppConfig {
    pub java: bool,
    pub java_hooks: Option<Vec<String>>,
    pub native: CoreNativeRole,
    pub app_hiding: bool,
    pub ports: bool,
    pub port_policy: Option<CorePortPolicy>,
    pub hidden: bool,
}
impl From<rpc::AppConfig> for CoreAppConfig {
    fn from(value: rpc::AppConfig) -> Self {
        Self {
            java: value.java,
            java_hooks: value.java_hooks,
            native: value.native.into(),
            app_hiding: value.app_hiding,
            ports: value.ports,
            port_policy: value.port_policy.map(|value| value.into()),
            hidden: value.hidden,
        }
    }
}
impl From<CoreAppConfig> for rpc::AppConfig {
    fn from(value: CoreAppConfig) -> Self {
        Self {
            java: value.java,
            java_hooks: value.java_hooks,
            native: value.native.into(),
            app_hiding: value.app_hiding,
            ports: value.ports,
            port_policy: value.port_policy.map(|value| value.into()),
            hidden: value.hidden,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeRole {
    pub enabled: bool,
    pub kernel: Option<Vec<String>>,
    pub zygisk: Option<Vec<String>>,
}
impl From<rpc::NativeRole> for CoreNativeRole {
    fn from(value: rpc::NativeRole) -> Self {
        Self {
            enabled: value.enabled,
            kernel: value.kernel,
            zygisk: value.zygisk,
        }
    }
}
impl From<CoreNativeRole> for rpc::NativeRole {
    fn from(value: CoreNativeRole) -> Self {
        Self {
            enabled: value.enabled,
            kernel: value.kernel,
            zygisk: value.zygisk,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CorePortPolicy {
    pub mode: CorePortPolicyMode,
    pub preset: Option<String>,
    pub rules: Vec<CorePortRule>,
}
impl From<rpc::PortPolicy> for CorePortPolicy {
    fn from(value: rpc::PortPolicy) -> Self {
        Self {
            mode: value.mode.into(),
            preset: value.preset,
            rules: value.rules.into_iter().map(|value| value.into()).collect(),
        }
    }
}
impl From<CorePortPolicy> for rpc::PortPolicy {
    fn from(value: CorePortPolicy) -> Self {
        Self {
            mode: value.mode.into(),
            preset: value.preset,
            rules: value.rules.into_iter().map(|value| value.into()).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CorePortRule {
    pub protocol: CorePortProtocol,
    pub start: u16,
    pub end: u16,
}
impl From<rpc::PortRule> for CorePortRule {
    fn from(value: rpc::PortRule) -> Self {
        Self {
            protocol: value.protocol.into(),
            start: value.start,
            end: value.end,
        }
    }
}
impl From<CorePortRule> for rpc::PortRule {
    fn from(value: CorePortRule) -> Self {
        Self {
            protocol: value.protocol.into(),
            start: value.start,
            end: value.end,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CorePortProtocol {
    Both,
    Tcp,
    Udp,
}
impl From<rpc::PortProtocol> for CorePortProtocol {
    fn from(value: rpc::PortProtocol) -> Self {
        match value {
            rpc::PortProtocol::Both => Self::Both,
            rpc::PortProtocol::Tcp => Self::Tcp,
            rpc::PortProtocol::Udp => Self::Udp,
        }
    }
}
impl From<CorePortProtocol> for rpc::PortProtocol {
    fn from(value: CorePortProtocol) -> Self {
        match value {
            CorePortProtocol::Both => Self::Both,
            CorePortProtocol::Tcp => Self::Tcp,
            CorePortProtocol::Udp => Self::Udp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreConfigToggle {
    Filesystem,
    Debug,
    DebugSwitch,
    RememberSuperkey,
    AutoHideServices,
    AutoHideName,
}
impl From<rpc::ConfigToggle> for CoreConfigToggle {
    fn from(value: rpc::ConfigToggle) -> Self {
        match value {
            rpc::ConfigToggle::Filesystem => Self::Filesystem,
            rpc::ConfigToggle::Debug => Self::Debug,
            rpc::ConfigToggle::DebugSwitch => Self::DebugSwitch,
            rpc::ConfigToggle::RememberSuperkey => Self::RememberSuperkey,
            rpc::ConfigToggle::AutoHideServices => Self::AutoHideServices,
            rpc::ConfigToggle::AutoHideName => Self::AutoHideName,
        }
    }
}
impl From<CoreConfigToggle> for rpc::ConfigToggle {
    fn from(value: CoreConfigToggle) -> Self {
        match value {
            CoreConfigToggle::Filesystem => Self::Filesystem,
            CoreConfigToggle::Debug => Self::Debug,
            CoreConfigToggle::DebugSwitch => Self::DebugSwitch,
            CoreConfigToggle::RememberSuperkey => Self::RememberSuperkey,
            CoreConfigToggle::AutoHideServices => Self::AutoHideServices,
            CoreConfigToggle::AutoHideName => Self::AutoHideName,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreConfigSetField {
    OptionalFeatures,
    AutoHideExcludedPackages,
    AutoHiddenPackages,
}
impl From<rpc::ConfigSetField> for CoreConfigSetField {
    fn from(value: rpc::ConfigSetField) -> Self {
        match value {
            rpc::ConfigSetField::OptionalFeatures => Self::OptionalFeatures,
            rpc::ConfigSetField::AutoHideExcludedPackages => Self::AutoHideExcludedPackages,
            rpc::ConfigSetField::AutoHiddenPackages => Self::AutoHiddenPackages,
        }
    }
}
impl From<CoreConfigSetField> for rpc::ConfigSetField {
    fn from(value: CoreConfigSetField) -> Self {
        match value {
            CoreConfigSetField::OptionalFeatures => Self::OptionalFeatures,
            CoreConfigSetField::AutoHideExcludedPackages => Self::AutoHideExcludedPackages,
            CoreConfigSetField::AutoHiddenPackages => Self::AutoHiddenPackages,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreConfigAppField {
    Java,
    Native,
    AppHiding,
    Ports,
    Hidden,
}
impl From<rpc::ConfigAppField> for CoreConfigAppField {
    fn from(value: rpc::ConfigAppField) -> Self {
        match value {
            rpc::ConfigAppField::Java => Self::Java,
            rpc::ConfigAppField::Native => Self::Native,
            rpc::ConfigAppField::AppHiding => Self::AppHiding,
            rpc::ConfigAppField::Ports => Self::Ports,
            rpc::ConfigAppField::Hidden => Self::Hidden,
        }
    }
}
impl From<CoreConfigAppField> for rpc::ConfigAppField {
    fn from(value: CoreConfigAppField) -> Self {
        match value {
            CoreConfigAppField::Java => Self::Java,
            CoreConfigAppField::Native => Self::Native,
            CoreConfigAppField::AppHiding => Self::AppHiding,
            CoreConfigAppField::Ports => Self::Ports,
            CoreConfigAppField::Hidden => Self::Hidden,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreObservationFreshness {
    Never,
    Current,
    Unknown,
}
impl From<rpc::ObservationFreshness> for CoreObservationFreshness {
    fn from(value: rpc::ObservationFreshness) -> Self {
        match value {
            rpc::ObservationFreshness::Never => Self::Never,
            rpc::ObservationFreshness::Current => Self::Current,
            rpc::ObservationFreshness::Unknown => Self::Unknown,
        }
    }
}
impl From<CoreObservationFreshness> for rpc::ObservationFreshness {
    fn from(value: CoreObservationFreshness) -> Self {
        match value {
            CoreObservationFreshness::Never => Self::Never,
            CoreObservationFreshness::Current => Self::Current,
            CoreObservationFreshness::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreFrameworkVpnStatus {
    Never,
    Classified,
    Unavailable,
    Failed,
}
impl From<rpc::FrameworkVpnStatus> for CoreFrameworkVpnStatus {
    fn from(value: rpc::FrameworkVpnStatus) -> Self {
        match value {
            rpc::FrameworkVpnStatus::Never => Self::Never,
            rpc::FrameworkVpnStatus::Classified => Self::Classified,
            rpc::FrameworkVpnStatus::Unavailable => Self::Unavailable,
            rpc::FrameworkVpnStatus::Failed => Self::Failed,
        }
    }
}
impl From<CoreFrameworkVpnStatus> for rpc::FrameworkVpnStatus {
    fn from(value: CoreFrameworkVpnStatus) -> Self {
        match value {
            CoreFrameworkVpnStatus::Never => Self::Never,
            CoreFrameworkVpnStatus::Classified => Self::Classified,
            CoreFrameworkVpnStatus::Unavailable => Self::Unavailable,
            CoreFrameworkVpnStatus::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreFrameworkVpnEvidence {
    pub status: CoreFrameworkVpnStatus,
    pub interfaces: Vec<String>,
    pub detail: String,
}
impl From<rpc::FrameworkVpnEvidence> for CoreFrameworkVpnEvidence {
    fn from(value: rpc::FrameworkVpnEvidence) -> Self {
        Self {
            status: value.status.into(),
            interfaces: value.interfaces,
            detail: value.detail,
        }
    }
}
impl From<CoreFrameworkVpnEvidence> for rpc::FrameworkVpnEvidence {
    fn from(value: CoreFrameworkVpnEvidence) -> Self {
        Self {
            status: value.status.into(),
            interfaces: value.interfaces,
            detail: value.detail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreUidVpnRouteState {
    VpnOff,
    Excluded,
    Routed,
    Unknown,
}
impl From<rpc::UidVpnRouteState> for CoreUidVpnRouteState {
    fn from(value: rpc::UidVpnRouteState) -> Self {
        match value {
            rpc::UidVpnRouteState::VpnOff => Self::VpnOff,
            rpc::UidVpnRouteState::Excluded => Self::Excluded,
            rpc::UidVpnRouteState::Routed => Self::Routed,
            rpc::UidVpnRouteState::Unknown => Self::Unknown,
        }
    }
}
impl From<CoreUidVpnRouteState> for rpc::UidVpnRouteState {
    fn from(value: CoreUidVpnRouteState) -> Self {
        match value {
            CoreUidVpnRouteState::VpnOff => Self::VpnOff,
            CoreUidVpnRouteState::Excluded => Self::Excluded,
            CoreUidVpnRouteState::Routed => Self::Routed,
            CoreUidVpnRouteState::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreUidRoutingObservation {
    pub uid: u32,
    pub state: CoreUidVpnRouteState,
    pub detail: String,
    pub session: Option<String>,
    pub interfaces: Vec<String>,
}
impl From<rpc::UidRoutingObservation> for CoreUidRoutingObservation {
    fn from(value: rpc::UidRoutingObservation) -> Self {
        Self {
            uid: value.uid,
            state: value.state.into(),
            detail: value.detail,
            session: value.session,
            interfaces: value.interfaces,
        }
    }
}
impl From<CoreUidRoutingObservation> for rpc::UidRoutingObservation {
    fn from(value: CoreUidRoutingObservation) -> Self {
        Self {
            uid: value.uid,
            state: value.state.into(),
            detail: value.detail,
            session: value.session,
            interfaces: value.interfaces,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreRoutingObservation {
    pub freshness: CoreObservationFreshness,
    pub generation: u64,
    pub vpn_interfaces: Vec<String>,
    pub last_good_vpn_interfaces: Vec<String>,
    pub netlink_candidates: Vec<String>,
    pub framework: CoreFrameworkVpnEvidence,
    pub uid: Option<CoreUidRoutingObservation>,
}
impl From<rpc::RoutingObservation> for CoreRoutingObservation {
    fn from(value: rpc::RoutingObservation) -> Self {
        Self {
            freshness: value.freshness.into(),
            generation: value.generation,
            vpn_interfaces: value.vpn_interfaces,
            last_good_vpn_interfaces: value.last_good_vpn_interfaces,
            netlink_candidates: value.netlink_candidates,
            framework: value.framework.into(),
            uid: value.uid.map(|value| value.into()),
        }
    }
}
impl From<CoreRoutingObservation> for rpc::RoutingObservation {
    fn from(value: CoreRoutingObservation) -> Self {
        Self {
            freshness: value.freshness.into(),
            generation: value.generation,
            vpn_interfaces: value.vpn_interfaces,
            last_good_vpn_interfaces: value.last_good_vpn_interfaces,
            netlink_candidates: value.netlink_candidates,
            framework: value.framework.into(),
            uid: value.uid.map(|value| value.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreNativeBackend {
    Kmod,
    Builtin,
    Kpm,
    Zygisk,
}
impl From<rpc::NativeBackend> for CoreNativeBackend {
    fn from(value: rpc::NativeBackend) -> Self {
        match value {
            rpc::NativeBackend::Kmod => Self::Kmod,
            rpc::NativeBackend::Builtin => Self::Builtin,
            rpc::NativeBackend::Kpm => Self::Kpm,
            rpc::NativeBackend::Zygisk => Self::Zygisk,
        }
    }
}
impl From<CoreNativeBackend> for rpc::NativeBackend {
    fn from(value: CoreNativeBackend) -> Self {
        match value {
            CoreNativeBackend::Kmod => Self::Kmod,
            CoreNativeBackend::Builtin => Self::Builtin,
            CoreNativeBackend::Kpm => Self::Kpm,
            CoreNativeBackend::Zygisk => Self::Zygisk,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeBackendStatus {
    pub observed_freshness: CoreObservationFreshness,
    pub selected: Option<CoreNativeBackend>,
    pub applied: Option<CoreNativeBackend>,
    pub active: Option<CoreNativeBackend>,
    pub installed: Vec<CoreNativeBackend>,
    pub boot: Option<CoreNativeBootStatus>,
}
impl From<rpc::NativeBackendStatus> for CoreNativeBackendStatus {
    fn from(value: rpc::NativeBackendStatus) -> Self {
        Self {
            observed_freshness: value.observed_freshness.into(),
            selected: value.selected.map(|value| value.into()),
            applied: value.applied.map(|value| value.into()),
            active: value.active.map(|value| value.into()),
            installed: value
                .installed
                .into_iter()
                .map(|value| value.into())
                .collect(),
            boot: value.boot.map(|value| value.into()),
        }
    }
}
impl From<CoreNativeBackendStatus> for rpc::NativeBackendStatus {
    fn from(value: CoreNativeBackendStatus) -> Self {
        Self {
            observed_freshness: value.observed_freshness.into(),
            selected: value.selected.map(|value| value.into()),
            applied: value.applied.map(|value| value.into()),
            active: value.active.map(|value| value.into()),
            installed: value
                .installed
                .into_iter()
                .map(|value| value.into())
                .collect(),
            boot: value.boot.map(|value| value.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreSelectionMode {
    Adopted,
    Loaded,
}
impl From<rpc::SelectionMode> for CoreSelectionMode {
    fn from(value: rpc::SelectionMode) -> Self {
        match value {
            rpc::SelectionMode::Adopted => Self::Adopted,
            rpc::SelectionMode::Loaded => Self::Loaded,
        }
    }
}
impl From<CoreSelectionMode> for rpc::SelectionMode {
    fn from(value: CoreSelectionMode) -> Self {
        match value {
            CoreSelectionMode::Adopted => Self::Adopted,
            CoreSelectionMode::Loaded => Self::Loaded,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeCapacityPackage {
    pub package: String,
    pub uids: Vec<u32>,
}
impl From<rpc::NativeCapacityPackage> for CoreNativeCapacityPackage {
    fn from(value: rpc::NativeCapacityPackage) -> Self {
        Self {
            package: value.package,
            uids: value.uids,
        }
    }
}
impl From<CoreNativeCapacityPackage> for rpc::NativeCapacityPackage {
    fn from(value: CoreNativeCapacityPackage) -> Self {
        Self {
            package: value.package,
            uids: value.uids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeCapacityReport {
    pub total: u32,
    pub capacity: u32,
    pub packages: Vec<CoreNativeCapacityPackage>,
}
impl From<rpc::NativeCapacityReport> for CoreNativeCapacityReport {
    fn from(value: rpc::NativeCapacityReport) -> Self {
        Self {
            total: value.total,
            capacity: value.capacity,
            packages: value
                .packages
                .into_iter()
                .map(|value| value.into())
                .collect(),
        }
    }
}
impl From<CoreNativeCapacityReport> for rpc::NativeCapacityReport {
    fn from(value: CoreNativeCapacityReport) -> Self {
        Self {
            total: value.total,
            capacity: value.capacity,
            packages: value
                .packages
                .into_iter()
                .map(|value| value.into())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreNativeBootOutcome {
    Selected {
        backend: CoreNativeBackend,
        mode: CoreSelectionMode,
        filesystem_hiding: Option<bool>,
    },
    Conflict {
        installed: Vec<CoreNativeBackend>,
        live: Vec<CoreNativeBackend>,
        staged: Vec<CoreNativeBackend>,
    },
    AwaitingAuthentication {
        detail: String,
    },
    KpmUnverifiable {
        detail: String,
    },
    KpmIncompatible {
        found: Option<String>,
        minimum: String,
        detail: String,
    },
    CapacityExceeded {
        backend: CoreNativeBackend,
        report: CoreNativeCapacityReport,
    },
    UnsupportedKernel {
        backend: CoreNativeBackend,
        uname: String,
        detail: String,
    },
    MissingArtifact {
        backend: CoreNativeBackend,
        path: String,
    },
    LoadFailed {
        backend: CoreNativeBackend,
        exit: Option<i32>,
        stderr: String,
        dmesg_tail: Vec<String>,
        kprobes: Option<bool>,
        kretprobes: Option<bool>,
        signature_rejected: bool,
    },
    DeliveryFailed {
        backend: CoreNativeBackend,
        detail: String,
    },
    ForeignOwner {
        id: u32,
    },
    NoBackend,
    Unknown {
        detail: String,
    },
}
impl From<rpc::NativeBootOutcome> for CoreNativeBootOutcome {
    fn from(value: rpc::NativeBootOutcome) -> Self {
        match value {
            rpc::NativeBootOutcome::Selected {
                backend,
                mode,
                filesystem_hiding,
            } => Self::Selected {
                backend: backend.into(),
                mode: mode.into(),
                filesystem_hiding,
            },
            rpc::NativeBootOutcome::Conflict {
                installed,
                live,
                staged,
            } => Self::Conflict {
                installed: installed.into_iter().map(|value| value.into()).collect(),
                live: live.into_iter().map(|value| value.into()).collect(),
                staged: staged.into_iter().map(|value| value.into()).collect(),
            },
            rpc::NativeBootOutcome::AwaitingAuthentication { detail } => {
                Self::AwaitingAuthentication { detail }
            }
            rpc::NativeBootOutcome::KpmUnverifiable { detail } => Self::KpmUnverifiable { detail },
            rpc::NativeBootOutcome::KpmIncompatible {
                found,
                minimum,
                detail,
            } => Self::KpmIncompatible {
                found,
                minimum,
                detail,
            },
            rpc::NativeBootOutcome::CapacityExceeded { backend, report } => {
                Self::CapacityExceeded {
                    backend: backend.into(),
                    report: report.into(),
                }
            }
            rpc::NativeBootOutcome::UnsupportedKernel {
                backend,
                uname,
                detail,
            } => Self::UnsupportedKernel {
                backend: backend.into(),
                uname,
                detail,
            },
            rpc::NativeBootOutcome::MissingArtifact { backend, path } => Self::MissingArtifact {
                backend: backend.into(),
                path,
            },
            rpc::NativeBootOutcome::LoadFailed {
                backend,
                exit,
                stderr,
                dmesg_tail,
                kprobes,
                kretprobes,
                signature_rejected,
            } => Self::LoadFailed {
                backend: backend.into(),
                exit,
                stderr,
                dmesg_tail,
                kprobes,
                kretprobes,
                signature_rejected,
            },
            rpc::NativeBootOutcome::DeliveryFailed { backend, detail } => Self::DeliveryFailed {
                backend: backend.into(),
                detail,
            },
            rpc::NativeBootOutcome::ForeignOwner { id } => Self::ForeignOwner { id },
            rpc::NativeBootOutcome::NoBackend => Self::NoBackend,
            rpc::NativeBootOutcome::Unknown { detail } => Self::Unknown { detail },
        }
    }
}
impl From<CoreNativeBootOutcome> for rpc::NativeBootOutcome {
    fn from(value: CoreNativeBootOutcome) -> Self {
        match value {
            CoreNativeBootOutcome::Selected {
                backend,
                mode,
                filesystem_hiding,
            } => Self::Selected {
                backend: backend.into(),
                mode: mode.into(),
                filesystem_hiding,
            },
            CoreNativeBootOutcome::Conflict {
                installed,
                live,
                staged,
            } => Self::Conflict {
                installed: installed.into_iter().map(|value| value.into()).collect(),
                live: live.into_iter().map(|value| value.into()).collect(),
                staged: staged.into_iter().map(|value| value.into()).collect(),
            },
            CoreNativeBootOutcome::AwaitingAuthentication { detail } => {
                Self::AwaitingAuthentication { detail }
            }
            CoreNativeBootOutcome::KpmUnverifiable { detail } => Self::KpmUnverifiable { detail },
            CoreNativeBootOutcome::KpmIncompatible {
                found,
                minimum,
                detail,
            } => Self::KpmIncompatible {
                found,
                minimum,
                detail,
            },
            CoreNativeBootOutcome::CapacityExceeded { backend, report } => Self::CapacityExceeded {
                backend: backend.into(),
                report: report.into(),
            },
            CoreNativeBootOutcome::UnsupportedKernel {
                backend,
                uname,
                detail,
            } => Self::UnsupportedKernel {
                backend: backend.into(),
                uname,
                detail,
            },
            CoreNativeBootOutcome::MissingArtifact { backend, path } => Self::MissingArtifact {
                backend: backend.into(),
                path,
            },
            CoreNativeBootOutcome::LoadFailed {
                backend,
                exit,
                stderr,
                dmesg_tail,
                kprobes,
                kretprobes,
                signature_rejected,
            } => Self::LoadFailed {
                backend: backend.into(),
                exit,
                stderr,
                dmesg_tail,
                kprobes,
                kretprobes,
                signature_rejected,
            },
            CoreNativeBootOutcome::DeliveryFailed { backend, detail } => Self::DeliveryFailed {
                backend: backend.into(),
                detail,
            },
            CoreNativeBootOutcome::ForeignOwner { id } => Self::ForeignOwner { id },
            CoreNativeBootOutcome::NoBackend => Self::NoBackend,
            CoreNativeBootOutcome::Unknown { detail } => Self::Unknown { detail },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreZygiskTargets {
    NotInstalled,
    Pending,
    Published,
    Retired,
    RetireFailed { detail: String },
}
impl From<rpc::ZygiskTargets> for CoreZygiskTargets {
    fn from(value: rpc::ZygiskTargets) -> Self {
        match value {
            rpc::ZygiskTargets::NotInstalled => Self::NotInstalled,
            rpc::ZygiskTargets::Pending => Self::Pending,
            rpc::ZygiskTargets::Published => Self::Published,
            rpc::ZygiskTargets::Retired => Self::Retired,
            rpc::ZygiskTargets::RetireFailed { detail } => Self::RetireFailed { detail },
        }
    }
}
impl From<CoreZygiskTargets> for rpc::ZygiskTargets {
    fn from(value: CoreZygiskTargets) -> Self {
        match value {
            CoreZygiskTargets::NotInstalled => Self::NotInstalled,
            CoreZygiskTargets::Pending => Self::Pending,
            CoreZygiskTargets::Published => Self::Published,
            CoreZygiskTargets::Retired => Self::Retired,
            CoreZygiskTargets::RetireFailed { detail } => Self::RetireFailed { detail },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreKmodBootReport {
    pub kprobes: Option<bool>,
    pub kretprobes: Option<bool>,
    pub gki_variant: String,
    pub version: String,
    pub uname: String,
}
impl From<rpc::KmodBootReport> for CoreKmodBootReport {
    fn from(value: rpc::KmodBootReport) -> Self {
        Self {
            kprobes: value.kprobes,
            kretprobes: value.kretprobes,
            gki_variant: value.gki_variant,
            version: value.version,
            uname: value.uname,
        }
    }
}
impl From<CoreKmodBootReport> for rpc::KmodBootReport {
    fn from(value: CoreKmodBootReport) -> Self {
        Self {
            kprobes: value.kprobes,
            kretprobes: value.kretprobes,
            gki_variant: value.gki_variant,
            version: value.version,
            uname: value.uname,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CorePortsSource {
    Boot,
    BootRetry,
    Edit,
}
impl From<rpc::PortsSource> for CorePortsSource {
    fn from(value: rpc::PortsSource) -> Self {
        match value {
            rpc::PortsSource::Boot => Self::Boot,
            rpc::PortsSource::BootRetry => Self::BootRetry,
            rpc::PortsSource::Edit => Self::Edit,
        }
    }
}
impl From<CorePortsSource> for rpc::PortsSource {
    fn from(value: CorePortsSource) -> Self {
        match value {
            CorePortsSource::Boot => Self::Boot,
            CorePortsSource::BootRetry => Self::BootRetry,
            CorePortsSource::Edit => Self::Edit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CorePortsStatus {
    pub source: CorePortsSource,
    pub ipv4: CoreFamilyApplyOutcome,
    pub ipv6: CoreFamilyApplyOutcome,
    pub target_count: u32,
    pub netd_ready: Option<bool>,
    pub detail: String,
    pub at_ms: u64,
}
impl From<rpc::PortsStatus> for CorePortsStatus {
    fn from(value: rpc::PortsStatus) -> Self {
        Self {
            source: value.source.into(),
            ipv4: value.ipv4.into(),
            ipv6: value.ipv6.into(),
            target_count: value.target_count,
            netd_ready: value.netd_ready,
            detail: value.detail,
            at_ms: value.at_ms,
        }
    }
}
impl From<CorePortsStatus> for rpc::PortsStatus {
    fn from(value: CorePortsStatus) -> Self {
        Self {
            source: value.source.into(),
            ipv4: value.ipv4.into(),
            ipv6: value.ipv6.into(),
            target_count: value.target_count,
            netd_ready: value.netd_ready,
            detail: value.detail,
            at_ms: value.at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreNativeBootStatus {
    pub boot_id: String,
    pub decided_epoch: u64,
    pub decided_at_ms: u64,
    pub updated_at_ms: u64,
    pub outcome: CoreNativeBootOutcome,
    pub installed: Vec<CoreNativeBackend>,
    pub live: Vec<CoreNativeBackend>,
    pub staged: Vec<CoreNativeBackend>,
    pub kmod: Option<CoreKmodBootReport>,
    pub zygisk_targets: CoreZygiskTargets,
    pub ports: Option<CorePortsStatus>,
    pub obsolete_modules: Vec<String>,
}
impl From<rpc::NativeBootStatus> for CoreNativeBootStatus {
    fn from(value: rpc::NativeBootStatus) -> Self {
        Self {
            boot_id: value.boot_id,
            decided_epoch: value.decided_epoch,
            decided_at_ms: value.decided_at_ms,
            updated_at_ms: value.updated_at_ms,
            outcome: value.outcome.into(),
            installed: value
                .installed
                .into_iter()
                .map(|value| value.into())
                .collect(),
            live: value.live.into_iter().map(|value| value.into()).collect(),
            staged: value.staged.into_iter().map(|value| value.into()).collect(),
            kmod: value.kmod.map(|value| value.into()),
            zygisk_targets: value.zygisk_targets.into(),
            ports: value.ports.map(|value| value.into()),
            obsolete_modules: value.obsolete_modules,
        }
    }
}
impl From<CoreNativeBootStatus> for rpc::NativeBootStatus {
    fn from(value: CoreNativeBootStatus) -> Self {
        Self {
            boot_id: value.boot_id,
            decided_epoch: value.decided_epoch,
            decided_at_ms: value.decided_at_ms,
            updated_at_ms: value.updated_at_ms,
            outcome: value.outcome.into(),
            installed: value
                .installed
                .into_iter()
                .map(|value| value.into())
                .collect(),
            live: value.live.into_iter().map(|value| value.into()).collect(),
            staged: value.staged.into_iter().map(|value| value.into()).collect(),
            kmod: value.kmod.map(|value| value.into()),
            zygisk_targets: value.zygisk_targets.into(),
            ports: value.ports.map(|value| value.into()),
            obsolete_modules: value.obsolete_modules,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreConfigPhase {
    Persist,
    Secret,
    Cleanup,
    Native,
    Ports,
}
impl From<rpc::ConfigPhase> for CoreConfigPhase {
    fn from(value: rpc::ConfigPhase) -> Self {
        match value {
            rpc::ConfigPhase::Persist => Self::Persist,
            rpc::ConfigPhase::Secret => Self::Secret,
            rpc::ConfigPhase::Cleanup => Self::Cleanup,
            rpc::ConfigPhase::Native => Self::Native,
            rpc::ConfigPhase::Ports => Self::Ports,
        }
    }
}
impl From<CoreConfigPhase> for rpc::ConfigPhase {
    fn from(value: CoreConfigPhase) -> Self {
        match value {
            CoreConfigPhase::Persist => Self::Persist,
            CoreConfigPhase::Secret => Self::Secret,
            CoreConfigPhase::Cleanup => Self::Cleanup,
            CoreConfigPhase::Native => Self::Native,
            CoreConfigPhase::Ports => Self::Ports,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CorePhaseRecord {
    pub phase: CoreConfigPhase,
    pub outcome: CorePhaseOutcome,
}
impl From<rpc::PhaseRecord> for CorePhaseRecord {
    fn from(value: rpc::PhaseRecord) -> Self {
        Self {
            phase: value.phase.into(),
            outcome: value.outcome.into(),
        }
    }
}
impl From<CorePhaseRecord> for rpc::PhaseRecord {
    fn from(value: CorePhaseRecord) -> Self {
        Self {
            phase: value.phase.into(),
            outcome: value.outcome.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CorePhaseOutcome {
    NotAttempted,
    Running,
    Confirmed,
    FailedKnown,
    Unknown,
}
impl From<rpc::PhaseOutcome> for CorePhaseOutcome {
    fn from(value: rpc::PhaseOutcome) -> Self {
        match value {
            rpc::PhaseOutcome::NotAttempted => Self::NotAttempted,
            rpc::PhaseOutcome::Running => Self::Running,
            rpc::PhaseOutcome::Confirmed => Self::Confirmed,
            rpc::PhaseOutcome::FailedKnown => Self::FailedKnown,
            rpc::PhaseOutcome::Unknown => Self::Unknown,
        }
    }
}
impl From<CorePhaseOutcome> for rpc::PhaseOutcome {
    fn from(value: CorePhaseOutcome) -> Self {
        match value {
            CorePhaseOutcome::NotAttempted => Self::NotAttempted,
            CorePhaseOutcome::Running => Self::Running,
            CorePhaseOutcome::Confirmed => Self::Confirmed,
            CorePhaseOutcome::FailedKnown => Self::FailedKnown,
            CorePhaseOutcome::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreFamilyApplyOutcome {
    Confirmed,
    FailedKnown,
    Unknown,
    NotAttempted,
}
impl From<rpc::FamilyApplyOutcome> for CoreFamilyApplyOutcome {
    fn from(value: rpc::FamilyApplyOutcome) -> Self {
        match value {
            rpc::FamilyApplyOutcome::Confirmed => Self::Confirmed,
            rpc::FamilyApplyOutcome::FailedKnown => Self::FailedKnown,
            rpc::FamilyApplyOutcome::Unknown => Self::Unknown,
            rpc::FamilyApplyOutcome::NotAttempted => Self::NotAttempted,
        }
    }
}
impl From<CoreFamilyApplyOutcome> for rpc::FamilyApplyOutcome {
    fn from(value: CoreFamilyApplyOutcome) -> Self {
        match value {
            CoreFamilyApplyOutcome::Confirmed => Self::Confirmed,
            CoreFamilyApplyOutcome::FailedKnown => Self::FailedKnown,
            CoreFamilyApplyOutcome::Unknown => Self::Unknown,
            CoreFamilyApplyOutcome::NotAttempted => Self::NotAttempted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CorePortsFamilyReport {
    pub ipv4: CoreFamilyApplyOutcome,
    pub ipv6: CoreFamilyApplyOutcome,
    pub target_count: u32,
}
impl From<rpc::PortsFamilyReport> for CorePortsFamilyReport {
    fn from(value: rpc::PortsFamilyReport) -> Self {
        Self {
            ipv4: value.ipv4.into(),
            ipv6: value.ipv6.into(),
            target_count: value.target_count,
        }
    }
}
impl From<CorePortsFamilyReport> for rpc::PortsFamilyReport {
    fn from(value: CorePortsFamilyReport) -> Self {
        Self {
            ipv4: value.ipv4.into(),
            ipv6: value.ipv6.into(),
            target_count: value.target_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreOperationRecord {
    pub op: u64,
    pub phases: Vec<CorePhaseRecord>,
    pub settled: Option<CorePhaseOutcome>,
    pub ports: Option<CorePortsFamilyReport>,
}
impl From<rpc::OperationRecord> for CoreOperationRecord {
    fn from(value: rpc::OperationRecord) -> Self {
        Self {
            op: value.op.0,
            phases: value.phases.into_iter().map(|value| value.into()).collect(),
            settled: value.settled.map(|value| value.into()),
            ports: value.ports.map(|value| value.into()),
        }
    }
}
impl From<CoreOperationRecord> for rpc::OperationRecord {
    fn from(value: CoreOperationRecord) -> Self {
        Self {
            op: rpc::OperationId(value.op),
            phases: value.phases.into_iter().map(|value| value.into()).collect(),
            settled: value.settled.map(|value| value.into()),
            ports: value.ports.map(|value| value.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreDiagnosticProbe {
    RootSnapshot { packages: bool },
    DebugSnapshot,
    HookCounters,
    Dmesg,
    LogcatAppTags,
    LogcatLsposedBoot,
    KernelPartitions,
    KernelImages,
    LsposedModulesDb,
    GroundTruthChecks,
    AppVpnState { uid: u32 },
}
impl From<rpc::DiagnosticProbe> for CoreDiagnosticProbe {
    fn from(value: rpc::DiagnosticProbe) -> Self {
        match value {
            rpc::DiagnosticProbe::RootSnapshot { packages } => Self::RootSnapshot { packages },
            rpc::DiagnosticProbe::DebugSnapshot => Self::DebugSnapshot,
            rpc::DiagnosticProbe::HookCounters => Self::HookCounters,
            rpc::DiagnosticProbe::Dmesg => Self::Dmesg,
            rpc::DiagnosticProbe::LogcatAppTags => Self::LogcatAppTags,
            rpc::DiagnosticProbe::LogcatLsposedBoot => Self::LogcatLsposedBoot,
            rpc::DiagnosticProbe::KernelPartitions => Self::KernelPartitions,
            rpc::DiagnosticProbe::KernelImages => Self::KernelImages,
            rpc::DiagnosticProbe::LsposedModulesDb => Self::LsposedModulesDb,
            rpc::DiagnosticProbe::GroundTruthChecks => Self::GroundTruthChecks,
            rpc::DiagnosticProbe::AppVpnState { uid } => Self::AppVpnState { uid },
        }
    }
}
impl From<CoreDiagnosticProbe> for rpc::DiagnosticProbe {
    fn from(value: CoreDiagnosticProbe) -> Self {
        match value {
            CoreDiagnosticProbe::RootSnapshot { packages } => Self::RootSnapshot { packages },
            CoreDiagnosticProbe::DebugSnapshot => Self::DebugSnapshot,
            CoreDiagnosticProbe::HookCounters => Self::HookCounters,
            CoreDiagnosticProbe::Dmesg => Self::Dmesg,
            CoreDiagnosticProbe::LogcatAppTags => Self::LogcatAppTags,
            CoreDiagnosticProbe::LogcatLsposedBoot => Self::LogcatLsposedBoot,
            CoreDiagnosticProbe::KernelPartitions => Self::KernelPartitions,
            CoreDiagnosticProbe::KernelImages => Self::KernelImages,
            CoreDiagnosticProbe::LsposedModulesDb => Self::LsposedModulesDb,
            CoreDiagnosticProbe::GroundTruthChecks => Self::GroundTruthChecks,
            CoreDiagnosticProbe::AppVpnState { uid } => Self::AppVpnState { uid },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreDiagnosticsRequest {
    pub probes: Vec<CoreDiagnosticProbe>,
    pub clear_dmesg: bool,
}
impl From<rpc::DiagnosticsRequest> for CoreDiagnosticsRequest {
    fn from(value: rpc::DiagnosticsRequest) -> Self {
        Self {
            probes: value.probes.into_iter().map(|value| value.into()).collect(),
            clear_dmesg: value.clear_dmesg,
        }
    }
}
impl From<CoreDiagnosticsRequest> for rpc::DiagnosticsRequest {
    fn from(value: CoreDiagnosticsRequest) -> Self {
        Self {
            probes: value.probes.into_iter().map(|value| value.into()).collect(),
            clear_dmesg: value.clear_dmesg,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreProbeOutput {
    pub probe: CoreDiagnosticProbe,
    pub exit: i32,
    pub output: String,
    pub file: Option<String>,
    pub artifacts: Vec<String>,
    pub truncated: bool,
}
impl From<rpc::ProbeOutput> for CoreProbeOutput {
    fn from(value: rpc::ProbeOutput) -> Self {
        Self {
            probe: value.probe.into(),
            exit: value.exit,
            output: value.output,
            file: value.file,
            artifacts: value.artifacts,
            truncated: value.truncated,
        }
    }
}
impl From<CoreProbeOutput> for rpc::ProbeOutput {
    fn from(value: CoreProbeOutput) -> Self {
        Self {
            probe: value.probe.into(),
            exit: value.exit,
            output: value.output,
            file: value.file,
            artifacts: value.artifacts,
            truncated: value.truncated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreDiagnosticsReport {
    pub outputs: Vec<CoreProbeOutput>,
    pub dmesg_cleared: bool,
}
impl From<rpc::DiagnosticsReport> for CoreDiagnosticsReport {
    fn from(value: rpc::DiagnosticsReport) -> Self {
        Self {
            outputs: value
                .outputs
                .into_iter()
                .map(|value| value.into())
                .collect(),
            dmesg_cleared: value.dmesg_cleared,
        }
    }
}
impl From<CoreDiagnosticsReport> for rpc::DiagnosticsReport {
    fn from(value: CoreDiagnosticsReport) -> Self {
        Self {
            outputs: value
                .outputs
                .into_iter()
                .map(|value| value.into())
                .collect(),
            dmesg_cleared: value.dmesg_cleared,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreLogcatAutoStopReason {
    SizeCap,
    DurationCap,
    LogcatExited,
    CoreRestarted,
}
impl From<rpc::LogcatAutoStopReason> for CoreLogcatAutoStopReason {
    fn from(value: rpc::LogcatAutoStopReason) -> Self {
        match value {
            rpc::LogcatAutoStopReason::SizeCap => Self::SizeCap,
            rpc::LogcatAutoStopReason::DurationCap => Self::DurationCap,
            rpc::LogcatAutoStopReason::LogcatExited => Self::LogcatExited,
            rpc::LogcatAutoStopReason::CoreRestarted => Self::CoreRestarted,
        }
    }
}
impl From<CoreLogcatAutoStopReason> for rpc::LogcatAutoStopReason {
    fn from(value: CoreLogcatAutoStopReason) -> Self {
        match value {
            CoreLogcatAutoStopReason::SizeCap => Self::SizeCap,
            CoreLogcatAutoStopReason::DurationCap => Self::DurationCap,
            CoreLogcatAutoStopReason::LogcatExited => Self::LogcatExited,
            CoreLogcatAutoStopReason::CoreRestarted => Self::CoreRestarted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreLogcatDebugRestore {
    Held,
    Pending,
    Restored,
    UserChoice,
}
impl From<rpc::LogcatDebugRestore> for CoreLogcatDebugRestore {
    fn from(value: rpc::LogcatDebugRestore) -> Self {
        match value {
            rpc::LogcatDebugRestore::Held => Self::Held,
            rpc::LogcatDebugRestore::Pending => Self::Pending,
            rpc::LogcatDebugRestore::Restored => Self::Restored,
            rpc::LogcatDebugRestore::UserChoice => Self::UserChoice,
        }
    }
}
impl From<CoreLogcatDebugRestore> for rpc::LogcatDebugRestore {
    fn from(value: CoreLogcatDebugRestore) -> Self {
        match value {
            CoreLogcatDebugRestore::Held => Self::Held,
            CoreLogcatDebugRestore::Pending => Self::Pending,
            CoreLogcatDebugRestore::Restored => Self::Restored,
            CoreLogcatDebugRestore::UserChoice => Self::UserChoice,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreLogcatDebugLease {
    pub previous_debug: bool,
    pub previous_debug_switch: bool,
    pub restore: CoreLogcatDebugRestore,
}
impl From<rpc::LogcatDebugLease> for CoreLogcatDebugLease {
    fn from(value: rpc::LogcatDebugLease) -> Self {
        Self {
            previous_debug: value.previous_debug,
            previous_debug_switch: value.previous_debug_switch,
            restore: value.restore.into(),
        }
    }
}
impl From<CoreLogcatDebugLease> for rpc::LogcatDebugLease {
    fn from(value: CoreLogcatDebugLease) -> Self {
        Self {
            previous_debug: value.previous_debug,
            previous_debug_switch: value.previous_debug_switch,
            restore: value.restore.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CoreLogcatSessionState {
    Recording,
    Stopped,
    AutoStopped { reason: CoreLogcatAutoStopReason },
}
impl From<rpc::LogcatSessionState> for CoreLogcatSessionState {
    fn from(value: rpc::LogcatSessionState) -> Self {
        match value {
            rpc::LogcatSessionState::Recording => Self::Recording,
            rpc::LogcatSessionState::Stopped => Self::Stopped,
            rpc::LogcatSessionState::AutoStopped { reason } => Self::AutoStopped {
                reason: reason.into(),
            },
        }
    }
}
impl From<CoreLogcatSessionState> for rpc::LogcatSessionState {
    fn from(value: CoreLogcatSessionState) -> Self {
        match value {
            CoreLogcatSessionState::Recording => Self::Recording,
            CoreLogcatSessionState::Stopped => Self::Stopped,
            CoreLogcatSessionState::AutoStopped { reason } => Self::AutoStopped {
                reason: reason.into(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreLogcatSessionStatus {
    pub session: u64,
    pub state: CoreLogcatSessionState,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub bytes: u64,
    pub truncated: bool,
    pub max_bytes: u64,
    pub max_duration_ms: u64,
    pub logcat_exit: Option<i32>,
    pub file: Option<String>,
    pub delivered: bool,
    pub debug: Option<CoreLogcatDebugLease>,
}
impl From<rpc::LogcatSessionStatus> for CoreLogcatSessionStatus {
    fn from(value: rpc::LogcatSessionStatus) -> Self {
        Self {
            session: value.session.0,
            state: value.state.into(),
            started_at_ms: value.started_at_ms,
            duration_ms: value.duration_ms,
            bytes: value.bytes,
            truncated: value.truncated,
            max_bytes: value.max_bytes,
            max_duration_ms: value.max_duration_ms,
            logcat_exit: value.logcat_exit,
            file: value.file,
            delivered: value.delivered,
            debug: value.debug.map(|value| value.into()),
        }
    }
}
impl From<CoreLogcatSessionStatus> for rpc::LogcatSessionStatus {
    fn from(value: CoreLogcatSessionStatus) -> Self {
        Self {
            session: rpc::LogcatSessionId(value.session),
            state: value.state.into(),
            started_at_ms: value.started_at_ms,
            duration_ms: value.duration_ms,
            bytes: value.bytes,
            truncated: value.truncated,
            max_bytes: value.max_bytes,
            max_duration_ms: value.max_duration_ms,
            logcat_exit: value.logcat_exit,
            file: value.file,
            delivered: value.delivered,
            debug: value.debug.map(|value| value.into()),
        }
    }
}

/// Preserve unknown imported values without treating them as a supported policy mode.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CorePortPolicyMode {
    Preset,
    Custom,
    Unknown { value: String },
}
impl From<String> for CorePortPolicyMode {
    fn from(value: String) -> Self {
        match value.as_str() {
            "preset" => Self::Preset,
            "custom" => Self::Custom,
            _ => Self::Unknown { value },
        }
    }
}
impl From<CorePortPolicyMode> for String {
    fn from(value: CorePortPolicyMode) -> Self {
        match value {
            CorePortPolicyMode::Preset => "preset".into(),
            CorePortPolicyMode::Custom => "custom".into(),
            CorePortPolicyMode::Unknown { value } => value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn canonical_projection_preserves_hook_overrides_and_unknown_imported_modes() {
        let config = rpc::CanonicalConfig {
            settings: rpc::Settings {
                optional_features: BTreeSet::from(["filesystem".into(), "future-feature".into()]),
                ..Default::default()
            },
            apps: BTreeMap::from([(
                "com.example".into(),
                rpc::AppConfig {
                    java: true,
                    java_hooks: Some(vec![]),
                    native: rpc::NativeRole {
                        enabled: true,
                        kernel: None,
                        zygisk: Some(vec!["getifaddrs".into()]),
                    },
                    port_policy: Some(rpc::PortPolicy {
                        mode: "imported-mode".into(),
                        preset: None,
                        rules: vec![rpc::PortRule {
                            protocol: rpc::PortProtocol::Tcp,
                            start: 443,
                            end: 443,
                        }],
                    }),
                    ports: true,
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let typed: CoreCanonicalConfig = config.clone().into();
        assert_eq!(
            typed.settings.optional_features,
            vec!["filesystem", "future-feature"]
        );
        assert_eq!(typed.apps["com.example"].java_hooks, Some(vec![]));
        assert_eq!(
            typed.apps["com.example"].port_policy.as_ref().unwrap().mode,
            CorePortPolicyMode::Unknown {
                value: "imported-mode".into()
            }
        );
        let restored: rpc::CanonicalConfig = typed.into();
        assert_eq!(restored, config);
    }
}
