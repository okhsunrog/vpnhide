# vpnhide -- LSPosed module + target picker app

Hooks `writeToParcel()` in `system_server` to strip VPN data before Binder serialization reaches target apps. Part of [vpnhide](../README.md).

The APK also serves as the **target management UI** for the entire vpnhide project.
It owns the canonical config at `/data/system/vpnhide_config.json`, then runs the
installed activators for the native and ports modules. LSPosed itself reads the
same JSON directly from `system_server`.

Zero presence in the target app's process -- only "System Framework" is needed in the LSPosed scope.

## What it hooks

`writeToParcel()` on three classes inside `system_server`:

| Class | Effect |
|---|---|
| `NetworkCapabilities` | VPN transport and capability flags stripped before serialization. Covers `hasTransport(VPN)`, `getAllNetworks()` + VPN scan, `getTransportInfo()`. |
| `NetworkInfo` | VPN type rewritten to WIFI before serialization |
| `LinkProperties` | VPN interface name and routes stripped before serialization |

Uses a ThreadLocal save/restore pattern so the original values are preserved for non-target callers.

### Per-UID filtering

Filtering is controlled by `Binder.getCallingUid()` -- only apps whose UID appears in the target list see the filtered view. System services, VPN clients, and everything else see real data.

### Target management

Targets are loaded from `/data/system/vpnhide_config.json`. A `FileObserver`
(inotify) watches that file and reloads immediately -- no reboot needed for
Java-layer target changes.

The hook resolves package names to appIds through `/data/system/packages.list`,
not PackageManager, because it runs inside PackageManager hooks and must avoid
re-entering them.

## Target picker app

The APK includes a Compose UI for managing target apps across all vpnhide modules:

- Lists all installed apps with icons, names, and package names
- Text search filter
- System apps toggle (selected system apps always visible)
- One row per app with the current roles:
  - **Java** — LSPosed Java/API hiding
  - **Native** — the active native backend (kmod, KPM, or Zygisk)
  - **Apps** — PackageManager observer mode
  - **Ports** — localhost-port blocking observer
- Save writes the canonical JSON via `su`, runs the active native module's
  activator (installed under `vpnhide_kmod`, `vpnhide_kpm`, or `vpnhide_zygisk`),
  and runs the ports activator if present. The LSPosed hook reloads the JSON itself.

Works on KernelSU, Magisk, and any other root solution.

## Install

1. Build the APK (`./gradlew assembleDebug`).
2. Install: `adb install app/build/outputs/apk/debug/app-debug.apk`.
3. Open LSPosed/Vector manager, go to Modules, enable **VPN Hide**.
4. Add **"System Framework"** to the module's scope. No other apps should be in scope.
5. Reboot.
6. Open the VPN Hide app and use the **Hiding** tab to manage roles for target apps.

## Combined use with native backends

For apps with aggressive anti-tamper SDKs, full VPN hiding requires covering both native and Java API paths without any hooks in the target app's process:

- **[kmod](../kmod/)** and **[KPM](../kmod/kpm/)** cover native paths at the kernel level: `ioctl`, `getifaddrs`/netlink, routes, and `/proc/net/route`.
- **This module** covers Java APIs: `NetworkCapabilities`, `NetworkInfo`, `LinkProperties` via `writeToParcel()` in `system_server`.

Together they provide complete VPN hiding with zero footprint in the target process.
Zygisk can be used as the native fallback, but its libc hooks run inside the
target process and can be detected by aggressive apps.

## Debugging

```bash
adb logcat | grep VpnHide
```

## Build

```bash
./gradlew assembleDebug
```

Requires JDK 17 or later. Output: `app/build/outputs/apk/debug/app-debug.apk`.

The build cross-compiles `lsposed/native/` (Rust crate) for `aarch64-linux-android` via cargo-ndk and bundles the resulting `libvpnhide_checks.so` into the APK's `jniLibs/`, plus auto-generated UniFFI Kotlin bindings under package `dev.okhsunrog.vpnhide.checks`. Both steps are driven by [Gobley](https://github.com/gobley/gobley) Gradle plugins (`dev.gobley.cargo` + `dev.gobley.uniffi`) — no manual `cargo` invocation needed. See [../docs/development.md](../docs/development.md#prerequisites) for the full prereq list.

## License

MIT. See [LICENSE](../LICENSE).
