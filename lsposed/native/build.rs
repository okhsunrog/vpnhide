//! Build-time glue for the lsposed native cdylib (`libvpnhide_checks.so`).
//!
//! Only purpose right now: align ELF LOAD segments on 16 KiB so the
//! library loads cleanly on 16 KiB-page Android devices (Pixel 8 Pro on
//! Android 16, and any future hardware that ships with 16 KiB pages by
//! default). NDK r28+ already does this by default, but passing the flag
//! explicitly keeps r27 builds compatible too — defence in depth.
//!
//! See: <https://developer.android.com/guide/practices/page-sizes>

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
    // liblog, for the panic hook's __android_log_write. Android-only: the host
    // test build has no such library.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-lib=log");
    }
}
