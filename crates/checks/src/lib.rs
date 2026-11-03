//! Bounded native observations for Core-issued fixtures.
//! The diagnostic model owns controls and assessment; collectors do not emit verdicts.
pub mod address;
pub mod binding;
pub mod diagnostic_raw;
pub mod filesystem;
pub mod fixture;
mod generated;
pub mod netlink_read;
pub mod netlink_wire;
pub mod proc_interfaces;
pub mod sys_interfaces;

/// The shared generated VPN-interface vocabulary; not diagnostic success evidence.
pub fn is_vpn_interface_name(name: &str) -> bool {
    generated::iface_lists::matches_vpn(name.as_bytes())
}
