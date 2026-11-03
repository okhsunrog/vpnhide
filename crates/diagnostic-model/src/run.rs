//! Complete required contract and run identity. The implemented fixture is a
//! subset; absent stimulus/observations never remove a required check.
use serde::{Deserialize, Serialize};

use crate::{NativeFixtureSpec, NativeMeasurement, ScopedNativeResult};

pub const DIAGNOSTIC_CONTRACT: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredCheck {
    IoctlFlags,
    IoctlMtu,
    Getifaddrs,
    NetlinkLinks,
    IoctlAddresses,
    BindToDevice,
    NetlinkAddresses,
    NetlinkRoutes,
    NetlinkRules,
    ProcRoutes,
    ProcInterfaces,
    SysInterfaces,
    FrameworkNetworks,
    FrameworkDefaultNetwork,
    FrameworkCapabilities,
    FrameworkLinkProperties,
    FrameworkLegacyInfo,
    FrameworkCallbacks,
    FrameworkPendingIntent,
    PackageVisibility,
    Ports,
    Filesystem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticPlan {
    pub contract: u32,
    pub required: Vec<RequiredCheck>,
}
impl DiagnosticPlan {
    /// Supported product contract + enabled optional features, independent of
    /// installed-hook telemetry. Missing hooks remain failed/unverified work.
    pub fn supported(filesystem: bool, packages: bool, ports: bool) -> Self {
        use RequiredCheck::*;
        let mut required = vec![
            IoctlFlags,
            IoctlMtu,
            Getifaddrs,
            NetlinkLinks,
            IoctlAddresses,
            BindToDevice,
            NetlinkAddresses,
            NetlinkRoutes,
            NetlinkRules,
            ProcRoutes,
            ProcInterfaces,
            SysInterfaces,
            FrameworkNetworks,
            FrameworkDefaultNetwork,
            FrameworkCapabilities,
            FrameworkLinkProperties,
            FrameworkLegacyInfo,
            FrameworkCallbacks,
            FrameworkPendingIntent,
        ];
        if filesystem {
            required.push(Filesystem);
        }
        if packages {
            required.push(PackageVisibility);
        }
        if ports {
            required.push(Ports);
        }
        required.sort();
        Self {
            contract: DIAGNOSTIC_CONTRACT,
            required,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticFingerprint {
    pub boot_id: String,
    pub core_epoch: u64,
    pub policy_sha256: String,
    /// Selection, delivery and loaded runtime identity; never a module path.
    pub runtime_sha256: String,
    /// Build + Connectivity APEX identity. Missing evidence disables reuse.
    pub platform_sha256: Option<String>,
    /// Live non-worker process identities and the generations actually read.
    /// Unknown is distinct from an observed empty process set.
    pub loaded_generations_sha256: Option<String>,
    pub loaded_generations_complete: bool,
}
impl DiagnosticFingerprint {
    pub fn reusable(&self) -> bool {
        !self.boot_id.is_empty()
            && self.core_epoch != 0
            && digest(&self.policy_sha256)
            && digest(&self.runtime_sha256)
            && self.loaded_generations_complete
            && self.platform_sha256.as_deref().is_some_and(digest)
            && self
                .loaded_generations_sha256
                .as_deref()
                .is_some_and(digest)
    }
}
fn digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticWorkerTask {
    pub session: u64,
    pub core_epoch: u64,
    pub fixture: NativeFixtureSpec,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticWorkerReport {
    pub session: u64,
    pub core_epoch: u64,
    pub native: NativeMeasurement,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticError {
    Disabled,
    Busy,
    WrongProcess,
    Expired,
    StaleSession,
    Malformed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DiagnosticOutcome {
    Running,
    /// Useful scoped evidence with remaining work. Never full protection PASS.
    Incomplete {
        native: ScopedNativeResult,
        missing: Vec<RequiredCheck>,
    },
    Infrastructure {
        detail: String,
        native: Option<ScopedNativeResult>,
        cleanup_confirmed: Option<bool>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticRun {
    #[serde(default)]
    pub addresses: Option<crate::ScopedAddressResult>,
    #[serde(default)]
    pub binding: Option<crate::ScopedBindResult>,
    pub session: u64,
    pub fingerprint: DiagnosticFingerprint,
    pub plan: DiagnosticPlan,
    pub outcome: DiagnosticOutcome,
    pub framework: Option<crate::framework::ScopedFrameworkResult>,
    pub stale: bool,
}
impl DiagnosticRun {
    fn refresh_missing(&mut self) {
        let assessment = self.assessment();
        if let DiagnosticOutcome::Incomplete { missing, .. } = &mut self.outcome {
            *missing = assessment
                .required
                .into_iter()
                .filter(|r| !assessment.proven.contains(r))
                .collect();
        }
    }
    pub fn record_addresses(&mut self, addresses: crate::ScopedAddressResult) {
        self.addresses = Some(addresses);
        self.refresh_missing();
    }
    pub fn record_binding(&mut self, binding: crate::ScopedBindResult) {
        self.binding = Some(binding);
        self.refresh_missing();
    }
    pub fn record_framework(&mut self, framework: crate::framework::ScopedFrameworkResult) {
        self.framework = Some(framework);
        self.refresh_missing();
    }
    pub fn record_native(&mut self, native: ScopedNativeResult) {
        self.outcome = DiagnosticOutcome::Incomplete {
            native,
            missing: Vec::new(),
        };
        self.refresh_missing();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NativeCheck;
    #[test]
    fn four_native_successes_never_replace_the_supported_contract() {
        let mut run = DiagnosticRun {
            addresses: None,
            binding: None,
            session: 1,
            framework: None,
            fingerprint: DiagnosticFingerprint {
                boot_id: "boot".into(),
                core_epoch: 9,
                policy_sha256: "a".repeat(64),
                runtime_sha256: "b".repeat(64),
                platform_sha256: None,
                loaded_generations_sha256: None,
                loaded_generations_complete: false,
            },
            plan: DiagnosticPlan::supported(true, true, true),
            outcome: DiagnosticOutcome::Running,
            stale: false,
        };
        assert!(!run.fingerprint.reusable());
        run.record_native(ScopedNativeResult::Proven {
            checks: vec![
                NativeCheck::IoctlFlags,
                NativeCheck::IoctlMtu,
                NativeCheck::Getifaddrs,
                NativeCheck::NetlinkGetlink,
            ],
        });
        let DiagnosticOutcome::Incomplete { missing, .. } = run.outcome else {
            panic!("partial run");
        };
        assert_eq!(missing.len(), 19);
        assert!(missing.contains(&RequiredCheck::Getifaddrs));
        assert!(missing.contains(&RequiredCheck::FrameworkCallbacks));
        assert!(missing.contains(&RequiredCheck::BindToDevice));
        assert!(missing.contains(&RequiredCheck::Filesystem));
    }
}
