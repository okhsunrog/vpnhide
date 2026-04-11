# vpnhide

Hide an active Android VPN connection from selected apps.

Three components work together to cover all detection vectors -- from Java APIs down to kernel syscalls. A diagnostic test app verifies everything works.

## Components

| Directory | What | How |
|-----------|------|-----|
| **[kmod/](kmod/)** | Kernel module (C) | `kretprobe` hooks in kernel space. Zero footprint in the target app's process -- invisible to any userspace anti-tamper SDK. |
| **[lsposed/](lsposed/)** | LSPosed module (Kotlin) | Hooks `writeToParcel` in `system_server` for per-UID Binder filtering. Only "System Framework" in LSPosed scope -- no in-process hooks. |
| **[zygisk/](zygisk/)** | Zygisk module (Rust) | Inline-hooks `libc.so` in the target app's process. Alternative to kmod for users who can't install a kernel module. |
| **[test-app/](test-app/)** | Diagnostic app (Kotlin + C++) | 22 checks covering all detection vectors. Logs to logcat under tag `VPNHideTest`. |

## Detection coverage

| # | Detection vector | SELinux | kmod | zygisk | lsposed |
|---|---|---|---|---|---|
| 1 | `ioctl(SIOCGIFFLAGS)` on tun0 | | x | x | |
| 2 | `ioctl(SIOCGIFNAME)` resolve index to name | | x | x | |
| 3 | `ioctl(SIOCGIFCONF)` interface enumeration | | x | x | |
| 4 | `getifaddrs()` (uses netlink internally) | | x | x | |
| 5 | netlink `RTM_GETLINK` dump | blocked | x | x | |
| 6 | netlink `RTM_GETADDR` dump (IPv4 + IPv6) | blocked | x | | |
| 7 | netlink `RTM_GETROUTE` dump | blocked | | | |
| 8 | `/proc/net/route` | blocked | x | x | |
| 9 | `/proc/net/ipv6_route` | blocked | | x | |
| 10 | `/proc/net/if_inet6` | blocked | | x | |
| 11 | `/proc/net/tcp`, `tcp6` | blocked | | | |
| 12 | `/proc/net/udp`, `udp6` | blocked | | | |
| 13 | `/proc/net/dev` | blocked | | | |
| 14 | `/proc/net/fib_trie` | blocked | | | |
| 15 | `/sys/class/net/tun0/` | blocked | | | |
| 16 | `NetworkCapabilities` (hasTransport, NOT_VPN, transportInfo) | | | | x |
| 17 | `NetworkInfo` (getType, getTypeName) | | | | x |
| 18 | `ConnectivityManager` (activeNetwork, allNetworks) | | | | x |
| 19 | `LinkProperties` (interfaceName, routes, DNS) | | | | x |
| 20 | `NetworkInterface.getNetworkInterfaces()` | | x | x | |
| 21 | `System.getProperty` (proxy settings) | | | x | |
| 22 | `/proc/net/route` via Java `FileInputStream` | blocked | x | x | |

**blocked** = SELinux denies access for untrusted apps (Android 10+). No hook needed.

**x** = actively filtered by this component.

Rows 1-4 and 20 are the only vectors reachable by regular apps. Everything else is either blocked by SELinux or goes through Java APIs (covered by lsposed).

## Which modules do I need?

- **Apps with aggressive anti-tamper SDKs** (banking, government): `kmod` + `lsposed`. Zero in-process footprint -- undetectable by integrity checks.
- **Other apps**: `zygisk` + `lsposed`. Simpler to install (no kernel module), covers all reachable vectors.
- **To verify your setup**: install `test-app`, add it to target lists, run with VPN active -- all checks should pass.

## Configuration

Both kmod and zygisk modules have a WebUI (KernelSU/Magisk manager -> module settings) to select target apps. On save, the WebUI writes to:
- `targets.txt` -- persistent package names (survives module updates)
- `/proc/vpnhide_targets` -- resolved UIDs for the kernel module (kmod only)
- `/data/system/vpnhide_uids.txt` -- resolved UIDs for the lsposed system_server hooks

All changes apply immediately -- no reboot needed.

## Building

- **kmod**: `cd kmod && make && ./build-zip.sh` (kernel source + clang, see [kmod/BUILDING.md](kmod/BUILDING.md))
- **zygisk**: `cd zygisk && ./build-zip.sh` (Rust + NDK + cargo-ndk)
- **lsposed**: `cd lsposed && ./gradlew assembleDebug` (JDK 17)
- **test-app**: `cd test-app && ./gradlew installDebug` (JDK 17 + NDK)

## Verified against

- [RKNHardering](https://github.com/xtclovver/RKNHardering/) -- all detection vectors clean
- [YourVPNDead](https://github.com/loop-uh/yourvpndead) -- all detection vectors clean

Both implement the official Russian Ministry of Digital Development VPN/proxy detection methodology ([source](https://t.me/ruitunion/893)).

## Split tunneling

Works correctly with split-tunnel VPN configurations. Only the apps in the target list are affected -- all other apps see normal VPN state.

Note: detection apps that compare device-reported public IP against external checkers require split tunneling -- the detection app's HTTPS requests must exit through the carrier, not the tunnel.

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
- Direct `svc #0` syscalls bypass zygisk's libc hooks (that's what kmod is for)
- Server-side detection is unfixable client-side -- use split tunneling

## License

- **zygisk**: 0BSD
- **lsposed**: unlicensed (do whatever you want)
- **kmod**: GPL-2.0 (required for kernel modules)
