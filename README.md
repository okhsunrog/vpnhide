# vpnhide

Hide an active Android VPN connection from selected apps.

## Which modules do I need?

You always need `lsposed` (handles Java API detection) plus one native module:

- **`kmod` + `lsposed`** (recommended) — kernel-level hooks, zero in-process footprint. Invisible to anti-tamper SDKs in banking/government apps. Requires a supported GKI kernel (see below).
- **`zygisk` + `lsposed`** — in-process libc hooks. Use this if your device's GKI generation isn't covered by the kmod builds, or if you can't install kernel modules.

## Install

Download the latest release from [Releases](https://github.com/okhsunrog/vpnhide/releases).

### kmod + lsposed (recommended)

1. Install `vpnhide-kmod-<your-gki>.zip` via KernelSU-Next manager → Modules → Install from storage
2. Install `vpnhide-lsposed.apk` as a regular app
3. In LSPosed manager, enable the vpnhide module and add **"System Framework"** to its scope
4. Reboot

**Finding your GKI generation:** run `adb shell uname -r`. The output looks like `6.1.75-android14-11-g...` — the generation is `android14-6.1`. Download the matching `vpnhide-kmod-android14-6.1.zip`.

> **Note:** the `android14` in the GKI name is NOT your Android version — it's the kernel generation. All Pixels from 6 to 9a share the same `android14-6.1` kernel. Pixel 10 series moves to `android16-6.12`.

### zygisk + lsposed

1. Install `vpnhide-zygisk.zip` via KernelSU-Next or Magisk manager → Modules
2. Install `vpnhide-lsposed.apk` as a regular app
3. In LSPosed manager, enable the vpnhide module and add **"System Framework"** to its scope
4. Reboot

## Configuration

Open the module's WebUI in KernelSU-Next or Magisk manager (tap the module → settings/WebUI). Select which apps should not see the VPN, then tap Save. Changes apply immediately — no reboot needed.

## Verify

Install `vpnhide-test.apk` from the release, add it to the target list via WebUI, and launch it with VPN active. All checks should show PASS.

## Components

| Directory | What | How |
|---|---|---|
| **[kmod/](kmod/)** | Kernel module (C) | `kretprobe` hooks in kernel space. Zero footprint in the target app's process. |
| **[lsposed/](lsposed/)** | LSPosed module (Kotlin) | Hooks `writeToParcel` in `system_server` for per-UID Binder filtering. No in-process hooks. |
| **[zygisk/](zygisk/)** | Zygisk module (Rust) | Inline-hooks `libc.so` in the target app's process. Alternative to kmod. |
| **[test-app/](test-app/)** | Diagnostic app (Kotlin + Rust) | 22 checks covering all detection vectors. |

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

Rows 1-4 and 20 are the only vectors reachable by regular apps. Everything else is either blocked by SELinux or goes through Java APIs (covered by lsposed).

## Building from source

- **kmod**: `cd kmod && make && ./build-zip.sh` — see [kmod/BUILDING.md](kmod/BUILDING.md)
- **zygisk**: `cd zygisk && ./build-zip.sh` (Rust + NDK + cargo-ndk)
- **lsposed**: `cd lsposed && ./gradlew assembleDebug` (JDK 17)
- **test-app**: `cd test-app && ./gradlew installDebug` (JDK 17 + Rust + NDK)

## Verified against

- [RKNHardering](https://github.com/xtclovver/RKNHardering/) — all detection vectors clean
- [YourVPNDead](https://github.com/loop-uh/yourvpndead) — all detection vectors clean

Both implement the official Russian Ministry of Digital Development VPN/proxy detection methodology ([source](https://t.me/ruitunion/893)).

## Split tunneling

Works correctly with split-tunnel VPN configurations. Only the apps in the target list are affected.

Detection apps that compare device-reported public IP against external checkers require split tunneling — the detection app's traffic must exit through the carrier, not the tunnel.

## Threat model

vpnhide hides an active VPN from specific apps. It is NOT designed for:
- Hiding root or custom ROM presence
- Bypassing Play Integrity
- Fooling server-side detection (DNS leakage, IP blocklists, latency/TLS fingerprinting)

## Known limitations

- `kmod` requires a GKI kernel with `CONFIG_KPROBES=y` (standard on Android 12+ devices)
- `lsposed` requires LSPosed, LSPosed-Next, or Vector
- `zygisk` is arm64 only
- Direct `svc #0` syscalls bypass zygisk's libc hooks — that's what kmod is for
- Server-side detection is unfixable client-side — use split tunneling

## License

MIT. See [LICENSE](LICENSE).

The kernel module declares `MODULE_LICENSE("GPL")` as required by the Linux kernel to resolve `EXPORT_SYMBOL_GPL` symbols at runtime.
