//! Core-owned autonomous fixture orchestration. Interface-only evidence is a
//! scoped result until the complete supported diagnostic plan is measured.

pub mod native_fixture;
pub mod route_fixture;

pub mod coordinator;

pub(crate) mod android_worker;

pub mod kernel_worker;

mod framework_fixture;

mod framework_run;
mod ports_fixture;

pub mod native_raw;

mod native_exchange;

use crate::boot_time;

pub(crate) fn raw_backend(backend: Option<vpnhide_core_rpc::NativeBackend>) -> Option<u32> {
    match backend {
        Some(vpnhide_core_rpc::NativeBackend::Kmod) => {
            Some(vpnhide_protocol::hook_ids::Backend::Kmod as u32)
        }
        Some(vpnhide_core_rpc::NativeBackend::Builtin) => {
            Some(vpnhide_protocol::hook_ids::Backend::Builtin as u32)
        }
        Some(vpnhide_core_rpc::NativeBackend::Kpm) => {
            Some(vpnhide_protocol::hook_ids::Backend::Kpm as u32)
        }
        _ => None,
    }
}
