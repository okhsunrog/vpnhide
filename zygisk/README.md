# vpnhide -- Zygisk module

Native-layer VPN interface hiding via inline libc hooks. Part of [vpnhide](../README.md).

## What it hooks

All hooks are inline on `libc.so` via ByteDance shadowhook:

| Hook | Detection path | What it does |
|------|---------------|--------------|
| `ioctl` | `SIOCGIFFLAGS` | Returns `ENODEV` for VPN interfaces (pre-screens input name). |
| `ioctl` | `SIOCGIFNAME` | Calls through; rewrites result to `ENODEV` if returned name is VPN. |
| `ioctl` | `SIOCGIFCONF` | Calls through; compacts VPN entries out of the returned `ifreq` array. |
| `getifaddrs` | `NetworkInterface.getNetworkInterfaces()`, Dart VM, direct C/C++ | Unlinks VPN entries from the returned linked list. |
| `openat` | `/proc/net/{route,ipv6_route,if_inet6,tcp,tcp6}` | Returns a memfd with VPN entries stripped out. |
| `recvmsg` | Netlink `RTM_NEWADDR` / `RTM_NEWLINK` dump responses | Removes VPN interface entries from netlink messages. |

## Architecture

### Why inline hooks instead of PLT

PLT hooks patch the caller library's procedure linkage table. At `post_app_specialize` time, `libflutter.so` / `libapp.so` / late-loaded JNI libraries are **not yet mapped** -- only ~350 Android system libraries are present, and none of them have PLT relocations for `ioctl` (the call sites are inside libc itself). Inline-hooking libc's entry points rewrites the function prologue in-place, so every caller in the process -- regardless of when it was loaded -- lands on our trampoline.

### Flow

1. **`pre_app_specialize`** -- runs on zygote before uid drop. Reads `args.nice_name`, checks against `/data/adb/vpnhide_zygisk/targets.txt`. Non-targeted apps get `DlCloseModuleLibrary` (zero cost after unload).
2. **`post_app_specialize`** -- on targeted processes only: `shadowhook_init`, install four inline hooks (`ioctl`, `getifaddrs`, `openat`, `recvmsg`), then scrub maps.

### Thread-local guard

The ioctl hook uses a thread-local `IN_GETIFADDRS` flag to pass through without filtering while libc's internal `getifaddrs` implementation is running. Without this, our `SIOCGIFFLAGS` filter returns `ENODEV` for VPN interfaces during libc's own ifaddrs list construction, which corrupts the list and breaks downstream consumers (including NFC/HCE payment flows).

### Maps scrubbing

After hook installation, `scrub_shadowhook_maps()` renames `[anon:shadowhook-island]` and `[anon:shadowhook-enter]` regions via `prctl(PR_SET_VMA, PR_SET_VMA_ANON_NAME, ..., "")`. This makes them show as plain `[anon:]` in `/proc/self/maps`, indistinguishable from hundreds of other anonymous mappings -- even to anti-tamper SDKs that read maps via raw `svc #0` syscalls.

### shadowhook fork

We carry a small fork at [okhsunrog/android-inline-hook](https://github.com/okhsunrog/android-inline-hook) (branch `vpnhide-zygisk`), vendored as a git submodule under `third_party/android-inline-hook/`, with two changes on top of upstream:

1. **`SHADOWHOOK_STATIC=ON`** -- builds `libshadowhook.a` instead of a shared library so it can be embedded directly into this Rust cdylib.
2. **`sh_linker_init()` stub** -- on Android 16 (API 36) the hardcoded symbol table in upstream's linker hook no longer matches the newer linker layout, causing `SHADOWHOOK_ERRNO_INIT_LINKER`. We don't need the deferred-hook feature (libc.so is always preloaded), so the stub skips this path entirely.

## Compatibility

The module declares Zygisk API v5 but only calls v1-era functions (`pre_app_specialize`, `post_app_specialize`, `args.nice_name`, `set_option(DlCloseModuleLibrary)`). The inline hooks happen via shadowhook inside the process, not through the Zygisk API.

| Setup | Works |
|-------|-------|
| Stock Magisk (API v5) + LSPosed | Yes |
| Magisk + ZygiskNext + LSPosed | Yes |
| Magisk + NeoZygisk + LSPosed | Yes |
| KernelSU + ZygiskNext + LSPosed | Yes |
| KernelSU-Next + NeoZygisk + LSPosed/Vector | Yes (tested baseline) |
| APatch + any Zygisk implementation + LSPosed | Yes (untested in CI) |

Hard requirements:

- arm64 / `aarch64-linux-android` only -- `build.rs` hard-fails on other targets.
- A Zygisk implementation that exposes API >= v1.
- [LSPosed/Vector](../lsposed/) for the Java-side companion.

## Build

Requirements:

- Rust >= 1.85 (edition 2024)
- `rustup target add aarch64-linux-android`
- `cargo install cargo-ndk`
- Android NDK (auto-detected under `~/Android/Sdk/ndk/`; any recent NDK that ships `libclang_rt.builtins-aarch64-android.a` works)
- CMake >= 3.22, Ninja
- `git submodule update --init --recursive`

Build and package:

```bash
./build-zip.sh
# Output: target/vpnhide-zygisk.zip (~180 KB)
```

`build.rs` invokes the NDK's CMake toolchain on the shadowhook submodule, pulls in `libclang_rt.builtins-aarch64-android.a` for `__clear_cache`, and statically links everything into `libvpnhide_zygisk.so`.

### Log level

Logging goes through the [`log`](https://crates.io/crates/log) crate + `android_logger`. The compile-time ceiling is controlled by a Cargo feature; calls below the ceiling are statically elided.

| Feature     | Default | Effect                          |
|-------------|---------|--------------------------------|
| `log-off`   |         | No logs at all                  |
| `log-error` |         | Errors only                     |
| `log-warn`  |         | Errors, warnings                |
| `log-info`  | Yes     | Errors, warnings, info          |
| `log-debug` |         | + debug (e.g. `on_load` traces) |
| `log-trace` |         | + trace                         |

Override the default:

```bash
cargo ndk -t arm64-v8a build --release \
  --no-default-features --features log-debug
```

## Install

1. `adb push target/vpnhide-zygisk.zip /sdcard/Download/`
2. KernelSU/Magisk manager -> Modules -> Install from storage -> pick the zip.
3. Reboot.
4. Pick target apps:
   - **VPN Hide app (recommended):** open the VPN Hide app (the [lsposed](../lsposed/) APK). Lists all installed apps with icons, search, and checkboxes. Works on both KernelSU and Magisk.
   - **Shell:** edit `/data/adb/vpnhide_zygisk/targets.txt` directly (one package name per line, `#` for comments). A base package name `com.example.app` also matches subprocesses like `com.example.app:background`.
5. Force-stop target apps: `adb shell am force-stop <pkg>`
6. Verify: `adb logcat | grep vpnhide-zygisk`

## Filter logic

VPN interface prefixes: `tun`, `ppp`, `tap`, `wg`, `ipsec`, `xfrm`, `utun`, `l2tp`, `gre`, plus anything containing the substring `vpn`. Matches the list in the [LSPosed companion](../lsposed/).

## Known limitations

- **Direct `svc #0` syscalls bypass the hook.** Apps issuing raw syscalls skip libc entirely. Use [vpnhide-kmod](../kmod/) for these apps.
- **arm64 only.** No 32-bit arm, no x86.
- **`getifaddrs` hook leaks a few bytes per call.** Unlinked VPN entries in the ifaddrs linked list are intentionally leaked rather than tracked with a shadow allocator. Acceptable tradeoff -- `getifaddrs` is called infrequently.
- **Tested on Android 16 (API 36).** Should work back to API 24 in principle, but nothing older has been exercised.

## Files

- `src/lib.rs` -- module entry point, target gating, hook installer, maps scrubbing
- `src/hooks.rs` -- hook replacements for ioctl, getifaddrs, openat, recvmsg
- `src/filter.rs` -- VPN interface name matching and proc/net content filters (unit tested)
- `src/shadowhook.rs` -- minimal FFI to shadowhook
- `build.rs` -- drives CMake on the shadowhook submodule
- `third_party/android-inline-hook/` -- submodule (our shadowhook fork)
- `module/` -- KernelSU/Magisk module metadata
- `build-zip.sh` -- cross-compile + package script

## License

MIT. See [LICENSE](../LICENSE).
