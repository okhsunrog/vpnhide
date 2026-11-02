<p align="center">
  <img src="assets/logo.png" width="200" alt="VPN Hide" />
</p>

<h1 align="center">VPN Hide</h1>

<p align="center">Hide an active Android VPN connection from selected apps.</p>

> **VPN Hide 2.0:** install **`vpnhide.apk`** and one **`vpnhide-core.zip`** from the same build. Core contains the kmod/KPM payloads and both Zygisk ABIs, selects and starts the applicable backend, writes configuration and applies Ports. The app uses a local socket and runs no `su`. No separate backend ZIP installation is required.

> **Main branch status:** the 2.0 refactor is ongoing. Automated maintenance, expanded autonomous diagnostics and device acceptance remain incomplete; see the [roadmap](ROADMAP.md).


<p align="center">
  <a href="https://github.com/okhsunrog/vpnhide/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/okhsunrog/vpnhide/ci.yml?label=CI" alt="CI"></a>
  <a href="https://github.com/okhsunrog/vpnhide/releases/latest"><img src="https://img.shields.io/github/v/release/okhsunrog/vpnhide" alt="Release"></a>
  <a href="https://github.com/okhsunrog/vpnhide/releases"><img src="https://img.shields.io/github/downloads/okhsunrog/vpnhide/total" alt="Downloads"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License"></a>
</p>

<p align="center"><strong><a href="README.md">Русская версия</a> · <a href="README.zh.md">中文版</a></strong></p>

## Why vpnhide over alternatives?

Existing modules like [NoVPNDetect](https://bitbucket.org/yuri-project/novpndetect) and [NoVPNDetect Enhanced](https://github.com/BlueCat300/NoVPNDetectEnhanced) only cover **Java API** detection and hook **inside the target app's process** via Xposed. This has two critical problems:

1. **Invisible to anti-tamper** — any app with memory injection checks detects the Xposed hooks and refuses to work. The NoVPNDetect Enhanced author explicitly states: *"The module will not work if the target app has LSPosed protection or memory injection checks. For example, MirPay, T-Bank."*
2. **No native coverage** — apps using C/C++ code, cross-platform frameworks (Flutter, React Native), or direct syscalls can detect VPN through `ioctl`, `getifaddrs`, netlink sockets, and `/proc/net/*`. These vectors are completely missed by Java-only hooks.

vpnhide solves both problems with a layered architecture:

**Layer 1 — Java API (lsposed module):** hooks `system_server`, not the target app. `NetworkCapabilities`, `NetworkInfo`, and `LinkProperties` are filtered at the Binder level *before* data reaches the app's process. The app receives clean data over IPC — no injection into its process, nothing for anti-tamper to detect. The same module also hides selected apps from observer apps at the `PackageManager` level (the **Apps** role).

**Layer 2 — Native (kmod, built-in, KPM, or Zygisk):** covers native detection paths. Exactly one native backend should be active:
- **kmod** (recommended for supported GKI kernels) — kernel-level `kprobe`/`kretprobe` hooks. Filters `ioctl`, `getifaddrs`/netlink interface, address, route, and policy-rule dumps, and rejects `SO_BINDTODEVICE` / `SO_BINDTOIFINDEX` for hidden interfaces before socket state changes. Zero footprint in the target process: no library injection, nothing to detect.
- **KPM** — a KernelPatch Module implementing the same 11 logical kernel hooks without a GKI-variant-specific `.ko`. Useful for old/non-GKI 4.14 / 4.19 / 5.4 kernels and cases where the `.ko` cannot load. Requires a KernelPatch runtime: APatch or [KPatch-Next-Module](https://github.com/KernelSU-Next/KPatch-Next-Module/releases/latest).
- **Zygisk** — fallback when a kernel-level backend is not possible. Its `libc.so` inline hooks include best-effort `setsockopt` filtering, run inside the target process, and can be bypassed by direct syscalls, so banking and anti-fraud apps may detect it. For those apps, leave Native off and rely on the Java layer.

**Layer 3 — Ports (part of the core module):** blocks selected apps from reaching `127.0.0.1` / `::1` (via iptables), so they can't detect a locally bound VPN / proxy daemon by its open port (the **Ports** role). The core module applies these rules; there is no separate Ports module in 2.0.

The target app's process is completely untouched when using LSPosed + a kernel-level Native backend (kmod or KPM) — no Xposed, no inline hooks, no modified memory regions. Because of that, vpnhide works with banking and government apps that actively detect and block Xposed-based modules.

## What vpnhide hides

vpnhide hides three things from selected apps, all configured per app via the four **J / N / A / P** roles (Java, Native, Apps, Ports):

1. **Interface hiding** — the main goal. It removes VPN interfaces and routes from native APIs (`ioctl`, `getifaddrs`, netlink, `/proc/net/*`, `NetworkInterface`), prevents binding sockets to a hidden interface, and sanitizes Java APIs (`NetworkCapabilities`, `NetworkInfo`, `LinkProperties`). It is delivered by two roles together — **Java (J)** and **Native (N)** — toggled independently.
2. **Port hiding** — blocks localhost access for selected apps so they cannot detect Clash, sing-box, V2Ray, Happ, and similar tools by probing local ports (the **Ports (P)** role).
3. **App hiding** — hides selected installed apps from selected observer apps. Useful against package visibility checks, for example when an app tries to determine whether a VPN or proxy client is installed (the **Apps (A)** role).

## Which modules do I need?

VPN Hide needs **the APK + one unified Core ZIP**. The Java layer additionally needs LSPosed/Vector with VPN Hide scoped to **System Framework**. Ports is part of Core.

Core checks applicability in order: compatible builtin driver, eligible bundled `.ko`, authenticated KPM runtime, then an eligible Zygisk loader. Multiple bundled payload files are alternatives, not installation conflicts or a user selection menu. Unknown compatibility or load state remains an explicit refusal reason.

A KernelPatch runtime for KPM and a Zygisk loader are separate system prerequisites when required by the chosen path. They are not separate VPN Hide backend ZIPs. The [builtin driver](builtin/README.md) also needs no companion backend module; Core is still required.

## Install

These instructions describe 2.0. Use the APK and complete Core ZIP from the same CI build or matching release; do not mix them with packages from the old installation model.

1. Install `vpnhide.apk`, then the complete `vpnhide-core.zip` through Magisk / KernelSU / APatch.
2. For Java, enable VPN Hide in LSPosed/Vector and add **System Framework** to its scope. Reboot.
3. Open **Dashboard**: Core reports backend selection, activation and missing prerequisites. No manual GKI choice or separate kmod/KPM/Zygisk ZIP download is needed.
4. In **Hiding**, assign **J / N / A / P** roles to target apps and save. Restart already-running targets when using Zygisk; a fresh diagnostic worker does not establish their hook generation.

Old separate modules are inventoried for migration/recovery. Prepare and verify the complete replacement before removing a working installation. The common automated maintenance flow remains under implementation.

Only Core writes `/data/system/vpnhide_config.json`, through typed app/agent commands. Isolated adapters read 0.x/1.x sources and preserve them until verified migration/archive.

## Screenshots

| Dashboard — VPN hidden | Hiding — one app list | How it works |
|:-:|:-:|:-:|
| <img src="assets/screenshots/dashboard-hidden.png" width="250"> | <img src="assets/screenshots/hiding-list.png" width="250"> | <img src="assets/screenshots/hiding-help.png" width="250"> |

| Statistics | Per-hook breakdown | Per-app hook selection |
|:-:|:-:|:-:|
| <img src="assets/screenshots/statistics.png" width="250"> | <img src="assets/screenshots/statistics-breakdown.png" width="250"> | <img src="assets/screenshots/hook-picker.png" width="250"> |

| Diagnostics | Settings | Community |
|:-:|:-:|:-:|
| <img src="assets/screenshots/diagnostics-native.png" width="250"> | <img src="assets/screenshots/settings.png" width="250"> | <img src="assets/screenshots/community.png" width="250"> |

## Verify

The app has a built-in diagnostics system that catches most setup problems automatically.

**Dashboard** (runs on every app launch):
- Module and backend status (installed, active, version, target count)
- LSPosed configuration validation — reads the LSPosed database to verify that VPN Hide is enabled, System Framework is in scope, and no extra apps are scoped (a common misconfiguration)
- Version mismatch detection — compares installed module versions with the running app version and tells you exactly what to update
- Native backend recommendation — detects your kernel and maps it to the right kmod, KPM, or Zygisk artifact
- Live hiding check (when VPN is active) — runs 13 native checks and 12 Java API checks to verify that VPN is actually hidden

Any issues found are shown as actionable cards with specific instructions.

**Statistics** tab — per-app breakdown of which apps probe for the VPN and how, showing which checks each app runs (counters reported by the active backends).

**Settings → Diagnostics** (Detailed diagnostics) — detailed per-check breakdown with individual PASS/FAIL results for all 25 checks. Useful for troubleshooting when the Dashboard shows incomplete hiding.

## Components

| Directory | What | How |
|---|---|---|
| **[core/](core/)** + **[crates/](crates/)** | VPN Hide Core module (Rust) | `vpnhide-core.zip`: the root daemon started once per boot by a one-line `service.sh`. It picks and loads exactly one native backend, delivers the config to it, applies the Ports iptables rules, serves the app's socket and runs the app's typed diagnostics. |
| **[builtin/](builtin/)** | Built-in kernel backend (C) | The same driver as the `.ko`, compiled into the kernel (`CONFIG_VPNHIDE=y`) for kernels the `.ko` cannot load on: integrated modules, no kprobes, LTO. `scripts/integrate.py` vendors it into a kernel tree; no module is needed, core adopts it at boot. |
| **[kmod/](kmod/)** | `.ko` kernel module + KPM backend (C) | Two kernel-level Native backends: the GKI `.ko` using `kretprobe`, and the KPM using KernelPatch inline hooks. Both have zero footprint in the target app's process; only one should be active. ([details](kmod/README.md), [KPM](kmod/kpm/README.md)) |
| **[lsposed/](lsposed/)** | LSPosed module + app (Kotlin + Rust) | Hooks `writeToParcel` in `system_server` for per-UID Binder filtering. The APK provides a dashboard (module status, version checks, LSPosed config validation, install recommendations), the Hiding tab for Java / Native / Apps / Ports roles, and diagnostics. ([details](lsposed/README.md)) |
| **[zygisk/](zygisk/)** | Zygisk module (Rust) | Inline-hooks `libc.so` in the target app's process. Fallback when a kernel-level backend is unavailable. ([details](zygisk/README.md)) |

## Detection coverage

| # | Detection vector | SELinux | kmod | KPM | Zygisk | LSPosed |
|---|---|---|---|---|---|---|
| 1 | `ioctl(SIOCGIFFLAGS)` on tun0 | | x | x | x | |
| 2 | `ioctl(SIOCGIFNAME)` resolve index to name | | x | x | x | |
| 3 | `ioctl(SIOCGIFMTU)` MTU fingerprinting | | x | x | x | |
| 4 | `ioctl(SIOCGIFCONF)` interface enumeration | | x | x | x | |
| 5 | All other `SIOCGIF*` (INDEX, HWADDR, ADDR, etc.) | | x | x | x | |
| 6 | `getifaddrs()` (uses netlink internally) | | x | x | x | |
| 7 | netlink `RTM_GETLINK` dump | | x | x | x | |
| 8 | netlink `RTM_GETADDR` dump (IPv4 + IPv6) | | x | x | x | |
| 9 | netlink `RTM_GETROUTE` dump | | x | x | x | |
| 10 | netlink `RTM_GETRULE` policy rules | | x | x | | |
| 11 | Public `/32` or `/128` host route to the VPN server | | x | x | | |
| 12 | `SO_BINDTODEVICE` / `SO_BINDTOIFINDEX` | | x | x | libc | |
| 13 | `/proc/net/route` | blocked | x | x | x | |
| 14 | `/proc/net/ipv6_route` | blocked | x | x | x | |
| 15 | `/proc/net/if_inet6` | blocked | | | x | |
| 16 | `/proc/net/tcp`, `tcp6` | blocked | | | x | |
| 17 | `/proc/net/udp`, `udp6` | blocked | | | | |
| 18 | `/proc/net/dev` | blocked | | | | |
| 19 | `/proc/net/fib_trie` | blocked | | | | |
| 20 | `/sys/class/net/tun0/` and `/proc/sys/net/*/{conf,neigh}/tun0` | blocked | opt-in | | | |
| 21 | `NetworkCapabilities` (hasTransport, NOT_VPN, transportInfo) | | | | | x |
| 22 | `NetworkInfo` (getType, getTypeName) | | | | | x |
| 23 | `ConnectivityManager.getActiveNetwork()` | | | | | x |
| 24 | `ConnectivityManager.getAllNetworks()` + VPN scan | | | | | x |
| 25 | `LinkProperties` (interfaceName) | | | | | x |
| 26 | `LinkProperties` (routes via VPN interfaces) | | | | | x |
| 27 | `NetworkInterface.getNetworkInterfaces()` | | x | x | x | |
| 28 | `/proc/net/route` via Java `FileInputStream` | blocked | x | x | x | |

**blocked** = on stock-enforcing builds (Android 10+) SELinux usually denies untrusted apps access to that `/proc/net/*` / `/sys` file. But **SELinux policy is configured differently across devices and ROMs** (OEM and custom ROMs, `permissive` builds), so only explicit coverage shown in the table should be treated as vpnhide protection.

**libc** = best-effort Zygisk coverage: a direct syscall bypasses the inline hook.

**opt-in** = the `.ko` filesystem-hiding feature is disabled by default. Enable
it in Settings and reboot; when disabled its global VFS probes are not installed.

Important: `ioctl` and netlink dumps are available to a regular app without help from SELinux; on Linux 5.7+, so is the first socket-interface bind. This is how detectors such as RKNHardering bypass the `/proc/net/route` denial through netlink (see [issue #86](https://github.com/okhsunrog/vpnhide/issues/86)). Kernel-level backends (kmod/KPM) cover the native paths marked above with no target-process footprint. Zygisk covers libc-routed calls only; a direct raw syscall bypasses its hooks. On older kernels, the kernel itself rejects an unprivileged interface bind. Everything else is either often SELinux-blocked on stock (device-dependent) or goes through Java APIs and is covered by LSPosed.

KPM implements the same 11 logical kernel hooks as the `.ko`, while the full vector map documents the remaining ABI and behavior differences on older kernels.

The full vector map — per-layer breakdown, SELinux caveats, and known gaps — lives in [docs/detection-vectors.md](docs/detection-vectors.md).

## Building from source

Component scripts produce internal assembly inputs. `core/build.py` produces only the daemon base, despite its local `vpnhide-core.zip` filename. `tools/vpnhide-tooling bundle assemble` assembles the installable package from that base, all seven KMIs, KPM and both Zygisk ABIs. Public release assets are only the complete Core, APK and signed package descriptor.

- **core**: `python3 core/build.py` — cross-compiles `vpnhide-core` for arm64 + armv7 (cargo-ndk) into `core/target/vpnhide-core.zip`.
- **kmod**: `./kmod/build.py --kmi android14-6.1` (or `--all`) — auto-spawns the DDK container via podman/docker. Full guide: [kmod/BUILDING.md](kmod/BUILDING.md).
- **KPM**: `python3 kmod/kpm/build.py` — builds the universal `vpnhide-kpm.zip` through the KernelPatch submodule. Details: [kmod/kpm/README.md](kmod/kpm/README.md).
- **zygisk**: `cd zygisk && ./build.py` (Rust + NDK + cargo-ndk)
- **lsposed**: `cd lsposed && ./gradlew assembleDebug` (JDK 17 + Rust + NDK + cargo-ndk)

### Notes for contributors stuck on Windows

If you're on Windows, there are some inconveniences with building some subprojects.

**lsposed**: builds fine in Android Studio.

For kmod and zygisk, you'll (unfortunately) need to install [Docker for Windows](https://docs.docker.com/desktop/setup/install/windows-install/).

**kmod**: `python .\kmod\build.py --kmi android14-6.1` — the script picks up Docker and pulls the same `ddk-min` image that CI uses.

**KPM**: build from Linux or WSL. The script expects POSIX tools, `make`/`clang`, the KernelPatch submodule, and the Android NDK; a native Windows build path is not documented.

**zygisk**:
```powershell
docker run --rm -it -v "${PWD}:/workspace" -v "vpnhide_cargo_cache:/usr/local/cargo/registry" -w /workspace ghcr.io/okhsunrog/vpnhide/ci:latest bash -c 'cd zygisk && python3 ./build.py'
```
The reason `zygisk` can't be built directly on Windows is that the `zygisk-api` dependency contains a file named `aux.rs`. Cargo uses `libgit2` for git operations, and `libgit2` refuses to create files whose names _contain_ reserved Windows device names (`AUX`, `CON`, `NUL`, …). You'll get: `cannot checkout to invalid path 'src/aux.rs'; class=Checkout (20)`. [Someone reports](https://superuser.com/a/1929659) that some Windows update made it possible to create files containing reserved words **with** an extension, but `libgit2` hasn't been updated to relax the guard.

## Verified against

- [RKNHardering](https://github.com/xtclovver/RKNHardering/) — all detection vectors clean
- [YourVPNDead](https://git.zapret.moe/zapretkvn/yourvpndead) ([GitHub mirror](https://github.com/zapret-discordyoutube/yourvpndead)) — all detection vectors clean

Both implement the official Russian Ministry of Digital Development VPN/proxy detection methodology ([source](https://t.me/ruitunion/893)).

## Split tunneling

Works correctly with split-tunnel VPN configurations: only the apps in the target list are affected.

Using split tunneling together with VPN Hide is strongly recommended. VPN Hide hides the VPN on the device but cannot change what the server sees: if an app goes through the tunnel, the service sees the VPN server's exit IP and can refuse on that alone. So apps that do not need the tunnel (banks, government services, payment apps) are best excluded from it in your VPN client — then they see your real IP and, with VPN Hide, no trace of the VPN. Details: [Direct access or through the tunnel](docs/help/en/split-tunneling.md).

Detection apps that compare the device-reported public IP against external checkers should stay outside the tunnel — their traffic should go through the carrier, not the VPN.

## Threat model

vpnhide hides an active VPN from specific apps. It is NOT designed for:
- Hiding root or custom ROM presence
- Bypassing Play Integrity
- Fooling server-side detection (DNS leakage, IP blocklists, latency/TLS fingerprinting)

## Known limitations

- `kmod` requires a supported GKI kernel with `CONFIG_KPROBES=y` (standard on Android 12+ devices)
- KPM requires a KernelPatch runtime (APatch or [KPatch-Next-Module](https://github.com/KernelSU-Next/KPatch-Next-Module/releases/latest)); do not install KPM together with the `.ko`
- `lsposed` requires LSPosed, LSPosed-Next, or Vector
- `zygisk` ships arm64-v8a and armeabi-v7a (32-bit injection); the kernel backends are arm64 only
- Direct `svc #0` syscalls bypass Zygisk's libc hooks — use a kernel-level backend (kmod or KPM) for that
- Server-side detection is unfixable client-side — use split tunneling
- Only the app (`dev.okhsunrog.vpnhide`) installed in the main Android user (user 0) can reach VPN Hide Core. Core binds its socket inside that one app's data directory, so copies installed in a secondary user, work profile or clone space show the "core not reachable" screen. Manage every profile from the main-user copy

## Support the project

vpnhide is free, with no ads and no telemetry. There will be no paid features — donating unlocks nothing.

The same list is in the app under **Settings → Support the project**, where addresses copy on tap.

**Boosty** — card payment, one-off or recurring: <https://boosty.to/okhsunrog/donate>

| Coin | Network | Address |
| --- | --- | --- |
| USDT | Tron (TRC20) | `TMskx2wKmPg11VYvHoS93vUQGm7yhcetUU` |
| BTC | Bitcoin | `bc1pmt9u6nux4x7n86zknwdgt9v02lah2tu6d983ak2prc5cwt8hsetq82ganh` |
| GRAM | The Open Network | `UQADYTtMBQdZvmNNEX02R9sACpdnXKlPV8RbuFrxo7JFBRGS` |
| LTC | Litecoin | `MBLKJfPNANH3U41UPJFtha7EPJGdbiW5dZ` |

## License

MIT. See [LICENSE](LICENSE).

The kernel module declares `MODULE_LICENSE("GPL")` as required by the Linux kernel to resolve `EXPORT_SYMBOL_GPL` symbols at runtime.

## Star History

<a href="https://www.star-history.com/?type=date&repos=okhsunrog%2Fvpnhide">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=okhsunrog/vpnhide&type=date&theme=dark&legend=top-left&sealed_token=L6VLoQFGmusCfaI01irFbE2MJWoOX9V4Z66YMzG6z0vD-xjku8IZX4jnDHFYeAjjEne48AgxfoSExLa90tYlqeYq7E32T0DGbdKrR8UTyp0zVCfDeKGdCIku_20sKVi9WBuO4aqa3nBKDnKepezie3AC67kmr-2mazo76SIUyXWpRp8Lb038KZtafha8" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=okhsunrog/vpnhide&type=date&legend=top-left&sealed_token=L6VLoQFGmusCfaI01irFbE2MJWoOX9V4Z66YMzG6z0vD-xjku8IZX4jnDHFYeAjjEne48AgxfoSExLa90tYlqeYq7E32T0DGbdKrR8UTyp0zVCfDeKGdCIku_20sKVi9WBuO4aqa3nBKDnKepezie3AC67kmr-2mazo76SIUyXWpRp8Lb038KZtafha8" />
    <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=okhsunrog/vpnhide&type=date&legend=top-left&sealed_token=L6VLoQFGmusCfaI01irFbE2MJWoOX9V4Z66YMzG6z0vD-xjku8IZX4jnDHFYeAjjEne48AgxfoSExLa90tYlqeYq7E32T0DGbdKrR8UTyp0zVCfDeKGdCIku_20sKVi9WBuO4aqa3nBKDnKepezie3AC67kmr-2mazo76SIUyXWpRp8Lb038KZtafha8" />
  </picture>
</a>
