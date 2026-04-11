# vpnhide

Hide an active Android VPN connection from selected apps.

Three components work together to cover all detection vectors -- from Java APIs down to kernel syscalls. A diagnostic test app verifies everything works.

## Components

| Directory | What | How |
|-----------|------|-----|
| **[zygisk/](zygisk/)** | Zygisk module (Rust) | Inline-hooks `libc.so` via [shadowhook](https://github.com/nicknisi/nicknisi): `ioctl`, `getifaddrs`, `openat` (`/proc/net/*`), `recvmsg` (netlink). Catches every caller regardless of load order. |
| **[lsposed/](lsposed/)** | LSPosed module (Kotlin) | Hooks Java network APIs (`NetworkCapabilities`, `NetworkInterface`, `LinkProperties`, etc.) and `writeToParcel` in `system_server` for cross-process Binder filtering. |
| **[kmod/](kmod/)** | Kernel module (C) | `kretprobe` hooks on `dev_ioctl`, `rtnl_fill_ifinfo`, `fib_route_seq_show`. Invisible to any userspace anti-tamper SDK. |
| **[test-app/](test-app/)** | Diagnostic app (Kotlin + C++) | 15 checks (6 native + 9 Java) covering all hook vectors. Logs everything to logcat under tag `VPNHideTest` for automated verification. |

## Which modules do I need?

- **Most apps**: `zygisk` + `lsposed`. Almost all apps check VPN status through both native and Java APIs, so both modules are needed.
- **Apps with aggressive anti-tamper SDKs**: `kmod` + `lsposed` (system_server mode). Some SDKs detect userspace hooks via raw `svc #0` syscalls and ELF integrity checks -- only kernel-level filtering is invisible to them.
- **To verify your setup**: install `test-app`, add it to target lists, run with VPN active -- all checks should be green.

## Configuration

All three modules share a target list. Use the WebUI (KernelSU/Magisk manager -> module settings) to select which apps should not see the VPN. The WebUI writes to:
- `targets.txt` -- package names (read by zygisk and lsposed)
- `/proc/vpnhide_targets` -- resolved UIDs (read by kmod)
- `/data/system/vpnhide_uids.txt` -- resolved UIDs (read by lsposed system_server hooks)

## Building

- **zygisk**: `cd zygisk && ./build-zip.sh` (Rust + NDK + cargo-ndk). See [zygisk/README.md](zygisk/README.md).
- **lsposed**: `cd lsposed && ./gradlew assembleDebug` (JDK 17). See [lsposed/README.md](lsposed/README.md).
- **kmod**: `cd kmod && ./build-zip.sh` (kernel source + cross-compiler). See [kmod/BUILDING.md](kmod/BUILDING.md).
- **test-app**: `cd test-app && ./gradlew assembleDebug` (JDK 17 + NDK for native checks).

## Verified against

- [RKNHardering](https://github.com/xtclovver/RKNHardering/) -- all detection vectors clean
- [YourVPNDead](https://github.com/loop-uh/yourvpndead) -- all detection vectors clean

Both implement the official Russian Ministry of Digital Development VPN/proxy detection methodology ([source](https://t.me/ruitunion/893)).

## Split tunneling

Works correctly with split-tunnel VPN configurations. Only the apps in the target list are affected -- all other apps see normal VPN state.

Note: detection apps that compare device-reported public IP against external checkers require split tunneling -- the detection app's HTTPS requests must exit through the carrier, not the tunnel. That is a network-layer fact, not something any client-side hook can fix.

## Threat model

vpnhide is designed for one scenario: "I have a VPN running and certain apps refuse to work because they detect it. I want those specific apps to think the VPN isn't there."

It is NOT designed for:
- Hiding root or custom ROM presence
- Bypassing Play Integrity
- Fooling server-side detection (DNS leakage, IP blocklists, latency fingerprinting, TLS fingerprinting)

## Known limitations

- `kmod` requires a GKI kernel with `CONFIG_KPROBES=y` (standard on Pixel 6-9a with `android14-6.1`)
- `lsposed` requires LSPosed or a compatible Xposed framework
- `zygisk` is arm64 only
- Direct `svc #0` syscalls bypass zygisk's libc hooks (that is what kmod is for)
- Server-side detection is unfixable client-side -- use split tunneling

## License

- **zygisk**: 0BSD
- **lsposed**: unlicensed (do whatever you want)
- **kmod**: GPL-2.0 (required for kernel modules)
