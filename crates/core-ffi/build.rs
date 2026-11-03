fn main() {
    // NDK defaults differ by architecture. Keep ARMv7 as well as ARM64 aligned
    // to the shared Android page contract, independently of the build adapter.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!(
            "cargo:rustc-link-arg=-Wl,-z,max-page-size={}",
            vpnhide_build_contract::ANDROID_MAX_PAGE_SIZE
        );
    }
}
