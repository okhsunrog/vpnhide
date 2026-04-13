<p align="center">
  <img src="branding/logo.png" width="200" alt="VPN Hide" />
</p>

<h1 align="center">VPN Hide</h1>

<p align="center">Hide an active Android VPN connection from selected apps.</p>

<p align="center"><strong><a href="README.ru.md">Русская версия</a></strong></p>

## Why vpnhide over alternatives?

Existing modules like [NoVPNDetect](https://bitbucket.org/yuri-project/novpndetect) and [NoVPNDetect Enhanced](https://github.com/BlueCat300/NoVPNDetectEnhanced) only cover **Java API** detection and hook **inside the target app's process** via Xposed. This has two critical problems:

1. **Invisible to anti-tamper** — any app with memory injection checks detects the Xposed hooks and refuses to work. The NoVPNDetect Enhanced author explicitly states: *"The module will not work if the target app has LSPosed protection or memory injection checks. For example, MirPay, T-Bank."*
2. **No native coverage** — apps using C/C++ code, cross-platform frameworks (Flutter, React Native), or direct syscalls can detect VPN through `ioctl`, `getifaddrs`, netlink sockets, and `/proc/net/*`. These vectors are completely missed by Java-only hooks.

vpnhide solves both problems with a two-layer architecture:

**Layer 1 — Java API (lsposed module):** hooks `system_server`, not the target app. `NetworkCapabilities`, `NetworkInfo`, and `LinkProperties` are filtered at the Binder level *before* data reaches the app's process. The app receives clean data over IPC — no injection into its process, nothing for anti-tamper to detect.

**Layer 2 — Native (kmod or zygisk):** covers every native detection path:
- **kmod** (recommended) — kernel-level `kretprobe` hooks. Filters `ioctl` (SIOCGIFFLAGS, SIOCGIFNAME, SIOCGIFCONF), `getifaddrs`/netlink dumps (RTM_GETLINK, RTM_GETADDR), and `/proc/net/*` reads — all before the syscall returns to userspace. Zero in-process footprint. No library injection. Nothing to detect.
- **zygisk** (alternative) — inline-hooks `libc.so` inside the app process. Same native coverage as kmod but runs in-process, so it's theoretically detectable by advanced anti-tamper. Use this if your kernel isn't supported by kmod.

The target app's process is completely untouched (with kmod + lsposed) — no Xposed, no inline hooks, no modified memory regions. This makes vpnhide work with MirPay, T-Bank, Alfa-Bank and other banking/government apps that actively detect and block Xposed-based modules.

## Which modules do I need?

You always need **lsposed** (Java API layer) plus one native module:

- **`kmod` + `lsposed`** (recommended) — fully out-of-process, invisible to anti-tamper. Requires a supported GKI kernel (see below).
- **`zygisk` + `lsposed`** — use this if your device's GKI generation isn't covered by the kmod builds, or if you can't install kernel modules.

## Install

Download the latest release from [Releases](https://github.com/okhsunrog/vpnhide/releases).

### kmod + lsposed (recommended)

1. Install `vpnhide-kmod-<your-gki>.zip` via KernelSU-Next manager → Modules → Install from storage
2. Install `vpnhide-lsposed.apk` as a regular app
3. In LSPosed manager, enable the vpnhide module and add **"System Framework"** to its scope
4. Reboot (required — LSPosed hooks are injected into `system_server` at boot, so the module must be active before `system_server` starts)
5. Open the VPN Hide app, grant it root access (Magisk will prompt automatically; on KernelSU-Next, grant permission manually in the manager), and select target apps

**How to find your GKI generation:**

1. On your phone, go to **Settings → About phone** and find the **Kernel version** line. It looks something like `6.1.75-android14-11-g...`
2. You need two parts from this string: the kernel version (`6.1`) and the android generation (`android14`). Together they form your GKI generation: `android14-6.1`
3. Download the matching file from the release: `vpnhide-kmod-android14-6.1.zip`

Alternatively, if you have ADB set up, run `adb shell uname -r` to see the same kernel version string.

> **Important:** the `android14` in the kernel string is NOT your Android version — it's the kernel generation. For example, Pixels from 6 to 9a all use the `android14-6.1` kernel regardless of whether they run Android 14 or 15. Pixel 10 series uses `android16-6.12`.

### zygisk + lsposed

1. Install `vpnhide-zygisk.zip` via KernelSU-Next or Magisk manager → Modules
2. Install `vpnhide-lsposed.apk` as a regular app
3. In LSPosed manager, enable the vpnhide module and add **"System Framework"** to its scope
4. Reboot (required — LSPosed hooks are injected into `system_server` at boot)
5. Open the VPN Hide app, grant it root access (Magisk will prompt automatically; on KernelSU-Next, grant permission manually in the manager), and select target apps

## Configuration

**VPN Hide app (recommended):** open the VPN Hide app (installed as `vpnhide-lsposed.apk`) and grant it root access (Magisk prompts automatically; on KernelSU-Next, grant permission in the manager). It shows all installed apps with icons, names, and search. Check the apps you want to hide VPN from, tap Save. Works with both kmod and zygisk — writes to all target locations automatically via `su`.

**Shell:** edit `/data/adb/vpnhide_kmod/targets.txt` or `/data/adb/vpnhide_zygisk/targets.txt` directly (one package name per line). Reboot for changes to take effect.

After changing targets, force-stop and restart the affected apps — hooks take effect on the next app launch.

## Verify

Open the VPN Hide app, switch to the Diagnostics tab, and run all checks with VPN active. The app auto-adds itself to the target list. All 26 checks should show PASS.

## Components

| Directory | What | How |
|---|---|---|
| **[kmod/](kmod/)** | Kernel module (C) | `kretprobe` hooks in kernel space. Zero footprint in the target app's process. ([details](kmod/README.md)) |
| **[lsposed/](lsposed/)** | LSPosed module + app (Kotlin + Rust) | Hooks `writeToParcel` in `system_server` for per-UID Binder filtering. The APK serves as target picker, diagnostics (26 checks), and module management UI. ([details](lsposed/README.md)) |
| **[zygisk/](zygisk/)** | Zygisk module (Rust) | Inline-hooks `libc.so` in the target app's process. Alternative to kmod. ([details](zygisk/README.md)) |

## Detection coverage

| # | Detection vector | SELinux | kmod | zygisk | lsposed |
|---|---|---|---|---|---|
| 1 | `ioctl(SIOCGIFFLAGS)` on tun0 | | x | x | |
| 2 | `ioctl(SIOCGIFNAME)` resolve index to name | | x | x | |
| 3 | `ioctl(SIOCGIFMTU)` MTU fingerprinting | | x | x | |
| 4 | `ioctl(SIOCGIFCONF)` interface enumeration | | x | x | |
| 5 | All other `SIOCGIF*` (INDEX, HWADDR, ADDR, etc.) | | x | x | |
| 6 | `getifaddrs()` (uses netlink internally) | | x | x | |
| 7 | netlink `RTM_GETLINK` dump | blocked | x | x | |
| 8 | netlink `RTM_GETADDR` dump (IPv4 + IPv6) | blocked | x | | |
| 9 | netlink `RTM_GETROUTE` dump | blocked | | | |
| 10 | `/proc/net/route` | blocked | x | x | |
| 11 | `/proc/net/ipv6_route` | blocked | | x | |
| 12 | `/proc/net/if_inet6` | blocked | | x | |
| 13 | `/proc/net/tcp`, `tcp6` | blocked | | | |
| 14 | `/proc/net/udp`, `udp6` | blocked | | | |
| 15 | `/proc/net/dev` | blocked | | | |
| 16 | `/proc/net/fib_trie` | blocked | | | |
| 17 | `/sys/class/net/tun0/` | blocked | | | |
| 18 | `NetworkCapabilities` (hasTransport, NOT_VPN, transportInfo) | | | | x |
| 19 | `NetworkInfo` (getType, getTypeName) | | | | x |
| 20 | `ConnectivityManager.getActiveNetwork()` | | | | x |
| 21 | `ConnectivityManager.getAllNetworks()` + VPN scan | | | | x |
| 22 | `LinkProperties` (interfaceName) | | | | x |
| 23 | `LinkProperties` (routes via VPN interfaces) | | | | x |
| 24 | `NetworkInterface.getNetworkInterfaces()` | | x | x | |
| 25 | `System.getProperty` (proxy settings) | | | x | |
| 26 | `/proc/net/route` via Java `FileInputStream` | blocked | x | x | |

**blocked** = SELinux denies access for untrusted apps (Android 10+). No hook needed.

Rows 1-6, 21, and 24 are the only vectors reachable by regular apps. Everything else is either blocked by SELinux or goes through Java APIs (covered by lsposed).

## Building from source

- **kmod**: `cd kmod && make && ./build-zip.sh` — see [kmod/BUILDING.md](kmod/BUILDING.md)
- **zygisk**: `cd zygisk && ./build-zip.sh` (Rust + NDK + cargo-ndk)
- **lsposed**: `cd lsposed && ./gradlew assembleDebug` (JDK 17 + Rust + NDK + cargo-ndk)

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
