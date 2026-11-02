use vpnhide_core_rpc as rpc;
macro_rules! mirror_enum {
    ($local:ident,$remote:ident,$($variant:ident),+ $(,)?) => {
        #[derive(Debug,Clone,PartialEq,Eq,uniffi::Enum)]
        pub enum $local {$($variant),+}
        impl From<rpc::$remote> for $local {
            fn from(value:rpc::$remote)->Self {match value {$(rpc::$remote::$variant=>Self::$variant),+}}
        }
    };
}
mirror_enum!(
    CoreZygiskPublication,
    ZygiskPublication,
    NotObserved,
    Matching,
    Absent,
    Different,
    Unknown,
    PriorBoot
);
mirror_enum!(
    CoreZygiskConsumption,
    ZygiskConsumption,
    Current,
    Stale,
    Unattributed
);
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreZygiskAuthorizationRef {
    pub operation: u64,
    pub boot_id: String,
    pub generation: u64,
    pub policy_sha256: String,
    pub directory_device: u64,
    pub directory_inode: u64,
}
impl From<rpc::ZygiskAuthorizationRef> for CoreZygiskAuthorizationRef {
    fn from(v: rpc::ZygiskAuthorizationRef) -> Self {
        Self {
            operation: v.operation,
            boot_id: v.boot_id,
            generation: v.generation,
            policy_sha256: v.policy_sha256,
            directory_device: v.directory_device,
            directory_inode: v.directory_inode,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreZygiskProcessConsumption {
    pub pid: u32,
    pub start_ticks: u64,
    pub uid: u32,
    pub process: String,
    pub abi: String,
    pub generation: Option<u64>,
    pub policy_sha256: Option<String>,
    pub consumption: CoreZygiskConsumption,
}
impl From<rpc::ZygiskProcessConsumption> for CoreZygiskProcessConsumption {
    fn from(v: rpc::ZygiskProcessConsumption) -> Self {
        Self {
            pid: v.pid,
            start_ticks: v.start_ticks,
            uid: v.uid,
            process: v.process,
            abi: v.abi,
            generation: v.generation,
            policy_sha256: v.policy_sha256,
            consumption: v.consumption.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CoreZygiskLifecycle {
    pub expected: CoreZygiskAuthorizationRef,
    pub core_epoch: u64,
    pub observed_at_boot_ms: Option<u64>,
    pub publication: CoreZygiskPublication,
    pub processes: Vec<CoreZygiskProcessConsumption>,
    pub process_inventory_complete: bool,
    pub hook_health_measured: bool,
}
impl From<rpc::ZygiskLifecycle> for CoreZygiskLifecycle {
    fn from(v: rpc::ZygiskLifecycle) -> Self {
        Self {
            expected: v.expected.into(),
            core_epoch: v.core_epoch,
            observed_at_boot_ms: v.observed_at_boot_ms,
            publication: v.publication.into(),
            processes: v.processes.into_iter().map(Into::into).collect(),
            process_inventory_complete: v.process_inventory_complete,
            hook_health_measured: v.hook_health_measured,
        }
    }
}
