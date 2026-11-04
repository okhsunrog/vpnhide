//! Portable build compatibility values. These are distinct from the frozen
//! backend control/telemetry wire in vpnhide_protocol. No runtime dependencies.
pub const RPC_VERSION: u32 = 33;
// Fences older writers from independently frozen managed policy recipients.
pub const JOURNAL_VERSION: u32 = 18;
pub const JOURNAL_MIN_READ: u32 = 1;

/// Shared Android ELF LOAD alignment for both app ABIs and the Core executable.
pub const ANDROID_MAX_PAGE_SIZE: u32 = 16_384;
