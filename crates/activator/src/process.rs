//! Compatibility export of the shared bounded direct-exec primitive.
//! Process capture has one owner; backend callers retain their existing API.
pub use vpnhide_bounded_process::{
    CapturedCommand, capture_command, output_with_input_timeout, output_with_timeout,
};
