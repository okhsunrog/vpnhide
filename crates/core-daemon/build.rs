fn main() {
    // Explicit for both Android ABIs: the ARMv7 linker otherwise defaults to
    // 4 KiB LOAD alignment. A 16 KiB maximum also works on 4 KiB-page devices.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!(
            "cargo:rustc-link-arg-bin=vpnhide-core=-Wl,-z,max-page-size={}",
            vpnhide_build_contract::ANDROID_MAX_PAGE_SIZE
        );
    }
}
