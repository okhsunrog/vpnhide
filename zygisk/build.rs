//! Build-time glue: compile ByteDance shadowhook (our fork) from source via
//! CMake and link the resulting static archive into this cdylib.
//!
//! The fork lives at `third_party/android-inline-hook` (git submodule,
//! branch `vpnhide-zygisk`) and adds two things on top of upstream:
//!
//!   1. a `SHADOWHOOK_STATIC=ON` CMake option that builds `libshadowhook.a`
//!      instead of a shared library and drops the SHARED-only link flags;
//!   2. a stub `sh_linker_init()` so `shadowhook_init()` doesn't fail on
//!      Android 16 with `SHADOWHOOK_ERRNO_INIT_LINKER`.
//!
//! See the fork's commit message for the full rationale.

use std::path::PathBuf;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=third_party/android-inline-hook/shadowhook/src/main/cpp");

    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("android") {
        // Host builds (cargo check / unit tests on linux-gnu) don't link
        // the Android cdylib; skip the whole native build step.
        return;
    }
    // Map the Rust target triple to the NDK's ANDROID_ABI name and the
    // compiler-rt builtins arch suffix. Both 64-bit (arm64-v8a) and 32-bit
    // (armeabi-v7a) are supported: shadowhook ships an inline-hook backend
    // for each (arch/arm64 and arch/arm), selected via ${ARCH} in its
    // CMakeLists, and the hook logic here is symbol-name based, so it's
    // ABI-agnostic. 32-bit is needed to inject into 32-bit app processes
    // (forked from zygote32) — otherwise NeoZygisk fails to load us there.
    let (android_abi, builtins_arch) = if target.starts_with("aarch64") {
        ("arm64-v8a", "aarch64")
    } else if target.starts_with("armv7") || target.starts_with("thumbv7") {
        ("armeabi-v7a", "arm")
    } else {
        panic!("vpnhide-zygisk supports arm64-v8a / armeabi-v7a only (target={target})");
    };

    // cargo-ndk sets ANDROID_NDK_HOME before invoking us.
    let ndk = env::var("ANDROID_NDK_HOME")
        .or_else(|_| env::var("ANDROID_NDK_ROOT"))
        .or_else(|_| env::var("NDK_HOME"))
        .expect("ANDROID_NDK_HOME must be set (cargo-ndk normally does this)");

    let toolchain_file = PathBuf::from(&ndk).join("build/cmake/android.toolchain.cmake");
    assert!(
        toolchain_file.is_file(),
        "NDK CMake toolchain not found at {}",
        toolchain_file.display()
    );

    let shadowhook_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("third_party/android-inline-hook/shadowhook/src/main/cpp");
    assert!(
        shadowhook_src.join("CMakeLists.txt").is_file(),
        "shadowhook submodule not initialized — run `git submodule update --init --recursive`"
    );

    // Build the static archive via the `cmake` crate. It picks up the NDK
    // toolchain and runs `cmake --build`. `no_build_target(true)` skips the
    // default `install` step (upstream has no install() rules) and lets us
    // pin the exact CMake target to build.
    let dst = cmake::Config::new(&shadowhook_src)
        .define("CMAKE_TOOLCHAIN_FILE", &toolchain_file)
        .define("ANDROID_ABI", android_abi)
        .define("ANDROID_PLATFORM", "android-24")
        .define("SHADOWHOOK_STATIC", "ON")
        .no_build_target(true)
        .build_target("shadowhook")
        .build();

    // cmake crate puts build artifacts under `<out>/build/`.
    let build_dir = dst.join("build");
    println!("cargo:rustc-link-search=native={}", build_dir.display());
    println!("cargo:rustc-link-lib=static=shadowhook");
    println!("cargo:rustc-link-lib=log");

    // Align ELF LOAD segments on 16 KiB so the cdylib loads cleanly on
    // 16 KiB-page Android devices (Pixel 8 Pro on Android 16 and any
    // future hardware that ships with 16 KiB pages by default). lld's
    // 4 KiB default would otherwise produce a startup warning, and on
    // future Androids may become a hard load failure.
    // See: <https://developer.android.com/guide/practices/page-sizes>
    println!(
        "cargo:rustc-link-arg=-Wl,-z,max-page-size={}",
        vpnhide_build_contract::ANDROID_MAX_PAGE_SIZE
    );

    // shadowhook's inline-patching code emits a libcall to `__clear_cache`
    // (I-cache flush after rewriting instructions). Rust's own
    // `compiler_builtins` doesn't export that symbol on aarch64-android,
    // so without this the cdylib has an unresolved reference and the
    // dynamic linker rejects it at dlopen time:
    //   cannot locate symbol "__clear_cache" referenced by ...
    // Pull it from the NDK's compiler-rt builtins archive.
    if let Some(builtins) = find_ndk_builtins(&ndk, builtins_arch) {
        println!("cargo:rustc-link-arg={}", builtins.display());
    } else {
        println!(
            "cargo:warning=libclang_rt.builtins-{builtins_arch}-android.a not found under {ndk}; \
             __clear_cache may be unresolved at dlopen time"
        );
    }
}

/// Locate `libclang_rt.builtins-<arch>-android.a` inside an NDK tree, where
/// `<arch>` is `aarch64` (arm64-v8a) or `arm` (armeabi-v7a).
/// Typical layout: `<ndk>/toolchains/llvm/prebuilt/<host>/lib/clang/<ver>/lib/linux/…`.
fn find_ndk_builtins(ndk: &str, arch: &str) -> Option<PathBuf> {
    let file_name = format!("libclang_rt.builtins-{arch}-android.a");
    let base = PathBuf::from(ndk).join("toolchains/llvm/prebuilt");
    for host in fs::read_dir(&base).ok()?.flatten() {
        let clang_dir = host.path().join("lib/clang");
        let Ok(versions) = fs::read_dir(&clang_dir) else {
            continue;
        };
        for v in versions.flatten() {
            let candidate = v.path().join("lib/linux").join(&file_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
