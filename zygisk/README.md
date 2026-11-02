# vpnhide -- Zygisk module

Native-layer VPN interface hiding via inline libc hooks. Part of [vpnhide](../README.md).

> **Using VPN Hide?** Install the matching APK and unified Core ZIP from [the main guide](../README.en.md#install). Both Zygisk ABI payloads are bundled in Core; no separate VPN Hide Zygisk ZIP is required.

## What it hooks

All hooks are inline on `libc.so` via ByteDance shadowhook:

| Hook | Detection path | What it does |
|------|---------------|--------------|
| `ioctl` | `SIOCGIFFLAGS` | Returns `ENODEV` for VPN interfaces (pre-screens input name). |
| `ioctl` | `SIOCGIFNAME` | Calls through; rewrites result to `ENODEV` if returned name is VPN. |
| `ioctl` | `SIOCGIFCONF` | Calls through; compacts VPN entries out of the returned `ifreq` array. |
| `getifaddrs` | `NetworkInterface.getNetworkInterfaces()`, Dart VM, direct C/C++ | Unlinks VPN entries from the returned linked list. |
| `setsockopt` | `SO_BINDTODEVICE`, `SO_BINDTOIFINDEX` | Best-effort fallback: returns `ENODEV` before the syscall for VPN names/indices. |
| `openat` | `/proc/net/{route,ipv6_route,if_inet6,tcp,tcp6}` | Returns a memfd with VPN entries stripped out. |
| `recvmsg` | Netlink `RTM_NEWADDR` / `RTM_NEWLINK` dump responses | Removes VPN interface entries from netlink messages. |
| `recv` | Netlink `RTM_NEWADDR` / `RTM_NEWLINK` dump responses | Same as `recvmsg`; hooked separately because bionic's `recv()` tail-calls `recvfrom`. |
| `recvfrom`, `__recvfrom_chk` | Direct/FORTIFY'd netlink reads | Same filtering for callers that bypass the plain `recv` symbol. |
| `open*`, `stat`/`lstat`/`fstat`/`fstatat`, `access*`, `readlink*`, `readdir*` | `/sys/.../net/<iface>` and `/proc/sys/net/{ipv4,ipv6}/{conf,neigh}/<iface>` | Optional best-effort group: returns `ENOENT` for VPN paths and removes VPN entries from directory listings. |

The filesystem group is controlled by the canonical
`settings.optionalFeatures: ["filesystem_iface_paths"]` setting. VPN Hide Core
projects hook bit 27 only into selected Zygisk target masks, and a force-stop +
restart is enough to apply a change. The group is installed atomically: if any
required libc symbol cannot be hooked, its stubs are rolled back, bit 27 is
omitted from the heartbeat's `installed_hooks`, and the ordinary network hooks
continue working.

## Architecture

### Why inline hooks instead of PLT

PLT hooks patch the caller library's procedure linkage table. At `post_app_specialize` time, `libflutter.so` / `libapp.so` / late-loaded JNI libraries are **not yet mapped** -- only ~350 Android system libraries are present, and none of them have PLT relocations for `ioctl` (the call sites are inside libc itself). Inline-hooking libc's entry points rewrites the function prologue in-place, so every caller in the process -- regardless of when it was loaded -- lands on our trampoline.

### Flow

1. **`pre_app_specialize`** -- runs in the already-forked child, before the kernel drops it to the app's UID and SELinux context (still has zygote privileges at this point). Reads `args.nice_name`, checks against the module-dir `targets.txt` runtime wire generated from `/data/system/vpnhide_config.json` by VPN Hide Core. Non-targeted apps get `DlCloseModuleLibrary` (zero cost after unload). See `src/lib.rs`'s top-level doc block for the full Zygisk lifecycle and why every Rust `static` is fresh per app launch.
2. **`post_app_specialize`** -- on targeted processes only: `shadowhook_init`, install the selected inline hooks (`ioctl`, `getifaddrs`, `setsockopt`, `openat`, `recvmsg`, `recv`, `recvfrom`, `__recvfrom_chk`, and the optional filesystem group), then scrub maps. `recv` is hooked separately because bionic's `recv()` is `b recvfrom` (tail-call) — patching `recvfrom`'s prologue would break `recv`.

### Thread-local guard

The ioctl hook uses a thread-local `IN_GETIFADDRS` flag to pass through without filtering while libc's internal `getifaddrs` implementation is running. Without this, our `SIOCGIFFLAGS` filter returns `ENODEV` for VPN interfaces during libc's own ifaddrs list construction, which corrupts the list and breaks downstream consumers (including NFC/HCE payment flows).

### Maps scrubbing

After hook installation, `scrub_shadowhook_maps()` renames `[anon:shadowhook-island]` and `[anon:shadowhook-enter]` regions via `prctl(PR_SET_VMA, PR_SET_VMA_ANON_NAME, ..., "")`. This makes them show as plain `[anon:]` in `/proc/self/maps`, indistinguishable from hundreds of other anonymous mappings -- even to anti-tamper SDKs that read maps via raw `svc #0` syscalls.

### shadowhook fork

We carry a small fork at [okhsunrog/android-inline-hook](https://github.com/okhsunrog/android-inline-hook) (branch `vpnhide-zygisk`), vendored as a git submodule under `third_party/android-inline-hook/`, with two changes on top of upstream:

1. **`SHADOWHOOK_STATIC=ON`** -- builds `libshadowhook.a` instead of a shared library so it can be embedded directly into this Rust cdylib.
2. **`sh_linker_init()` stub** -- on Android 16 (API 36) the hardcoded symbol table in upstream's linker hook no longer matches the newer linker layout, causing `SHADOWHOOK_ERRNO_INIT_LINKER`. We don't need the deferred-hook feature (libc.so is always preloaded), so the stub skips this path entirely.

## Compatibility

The module declares Zygisk API v2 but only calls v1-era functions (`pre_app_specialize`, `post_app_specialize`, `args.nice_name`, `set_option(DlCloseModuleLibrary)`). The inline hooks happen via shadowhook inside the process, not through the Zygisk API.

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

- Rust 1.99.1 (edition 2024), pinned by the workspace `rust-toolchain.toml`
- `rustup show` (installs the pinned toolchain and Android targets)
- `cargo install cargo-ndk`
- Android NDK (auto-detected under `~/Android/Sdk/ndk/`; any recent NDK that ships `libclang_rt.builtins-aarch64-android.a` works)
- CMake >= 3.22, Ninja
- `git submodule update --init --recursive`

Build and package:

```bash
./build.py
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

## Delivery inside Core

Install the unified Core package and provide a compatible Zygisk loader when
needed. Core verifies loader evidence, selects this backend and publishes its
current-boot authorization before launching the controlled worker. Bundled loader
libraries alone are not authorization to install hooks. Existing target processes
retain their consumed generation until they restart.

`target/vpnhide-zygisk.zip` is an internal assembly input. Do not install it as a
second backend module alongside Core. Device/ABI acceptance remains separate from
successful compilation; see the [implementation ledger](../docs/notes/unified-core-implementation.md).

## Filter logic

VPN interface prefixes: `tun`, `ppp`, `tap`, `wg`, `ipsec`, `xfrm`, `utun`, `l2tp`, `gre`, `tailscale`, `zt`, `he-ipv6`, plus anything containing the substring `vpn`, plus `if<N>` renamed/anonymous netdevs. Matches the list in the [LSPosed companion](../lsposed/).

## Known limitations

- **Direct `svc #0` syscalls bypass the hook.** Apps issuing raw syscalls skip libc entirely, including the best-effort `setsockopt` protection. Use a kernel-level backend, [vpnhide-kmod](../kmod/) or [KPM](../kmod/kpm/), for these apps.
- **Filesystem hiding is best-effort.** The optional group handles canonical
  and relative libc paths, but raw `openat`/`getdents64` syscalls, unhooked libc
  aliases, bind mounts, and paths reached through unrelated symlink aliases can
  bypass it. The kernel backends classify resolved dentries and provide the
  stronger contract.
- **Socket binding is left native before Linux 5.7.** Those kernels reject an unprivileged `SO_BINDTODEVICE` before reading the name. Android common 5.4 backports `SO_BINDTOIFINDEX` with the same capability gate; 4.x lacks it. Filtering would introduce a distinguishable error instead of closing an oracle.
- **arm64 only.** No 32-bit arm, no x86.
- **`getifaddrs` hook leaks a few bytes per call.** Unlinked VPN entries in the ifaddrs linked list are intentionally leaked rather than tracked with a shadow allocator. Acceptable tradeoff -- `getifaddrs` is called infrequently.
- **Tested on Android 16 (API 36).** Should work back to API 24 in principle, but nothing older has been exercised.

## Files

- `src/lib.rs` -- module entry point, target gating, hook installer, maps scrubbing
- `src/hooks.rs` -- hook replacements for ioctl, getifaddrs, setsockopt, openat, and the recv family
- `src/filesystem.rs` -- optional best-effort libc filesystem path and directory filtering
- `src/filter.rs` -- VPN interface name matching and proc/net content filters (unit tested)
- `src/shadowhook.rs` -- minimal FFI to shadowhook
- `build.rs` -- drives CMake on the shadowhook submodule
- `third_party/android-inline-hook/` -- submodule (our shadowhook fork)
- `module/` -- KernelSU/Magisk module metadata
- `build.py` -- cross-compile + package script

<details>
<summary><strong>For developers / advanced usage</strong></summary>

You do not need this to use the module — the VPN Hide app saves the config
through VPN Hide Core, which publishes the selected authenticated policy. After a Save, force-stop the target so it
re-forks with the new config:

```sh
adb shell am force-stop <pkg>
```

</details>

## Core delivery (2.0)

The complete Core ZIP carries both manager-discoverable Zygisk libraries and
verified payload copies. Core owns the selected full policy, durable grant/revoke
lifecycle and exact process-generation observations. A published authorization
or fresh worker heartbeat does not prove every existing game has consumed it.
Revocation does not unload hooks already resident in those processes, so safe
transitions account for possible exposure and may require reboot.

Old module-dir readers are isolated recovery/compatibility adapters, not the
normal installation model. See the [Core architecture](../docs/core-architecture.md).

## License

MIT. See [LICENSE](../LICENSE).
