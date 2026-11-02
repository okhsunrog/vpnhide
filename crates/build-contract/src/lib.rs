//! Portable build compatibility values. These are distinct from the frozen
//! backend control/telemetry wire in vpnhide_protocol. No runtime dependencies.
pub const RPC_VERSION: u32 = 23;
pub const JOURNAL_VERSION: u32 = 11;
pub const JOURNAL_MIN_READ: u32 = 1;
