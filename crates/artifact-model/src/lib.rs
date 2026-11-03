//! Portable artifact formats and validation shared by device readers and host tools.
//! No root daemon, Android orchestration or publisher-label trust.
#[cfg(feature = "archives")]
pub mod archive;
pub mod core_layout;
#[cfg(feature = "archives")]
pub mod distribution;
pub mod elf;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
#[cfg(feature = "archives")]
pub mod initramfs;
pub mod manifest;
pub mod package;
pub use manifest::MAX_PAYLOAD_BYTES;
fn invalid(error: impl ToString) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
}
