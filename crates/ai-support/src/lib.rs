//! Server-side support assistant for VPN Hide.
//!
//! The APK sends a compact diagnostic snapshot and a question. This process
//! owns the system prompt, the model choice, and the only tools the model may
//! call. It is not the on-device core daemon and it does not activate hiding.
//!
//! This crate is a sketch. Model calls, the GitHub App, and private archive
//! storage are intentionally absent.

/// Crate name, stable for logs.
pub const CRATE_NAME: &str = "vpnhide_ai_support";

pub mod limits;
pub mod snapshot;
pub mod tools;
