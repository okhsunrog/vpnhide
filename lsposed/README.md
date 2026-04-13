# vpnhide -- LSPosed module + target picker app

Hooks `writeToParcel()` in `system_server` to strip VPN data before Binder serialization reaches target apps. Part of [vpnhide](../README.md).

The APK also serves as the **target management UI** for the entire vpnhide project — it writes targets for both [kmod](../kmod/) and [zygisk](../zygisk/) modules.

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

Target UIDs are loaded from `/data/system/vpnhide_uids.txt`. A `FileObserver` (inotify) watches for changes and reloads the list immediately -- no reboot needed.

This file is written by:
- The **VPN Hide app** (this APK's target picker UI)
- The module's `service.sh` on boot

## Target picker app

The APK includes a Compose UI for managing target apps across all vpnhide modules:

- Lists all installed apps with icons, names, and package names
- Text search filter
- System apps toggle (selected system apps always visible)
- Save writes to all target locations via `su`:
  - `/data/adb/vpnhide_kmod/targets.txt` (if kmod is installed)
  - `/data/adb/vpnhide_zygisk/targets.txt` (if zygisk is installed)
  - `/data/adb/modules/vpnhide_zygisk/targets.txt` (Magisk module dir copy)
  - `/proc/vpnhide_targets` (kmod live update, no reboot needed)
  - `/data/system/vpnhide_uids.txt` (system_server hooks, live reload via inotify)

Works on KernelSU, Magisk, and any other root solution.

## Install

1. Build the APK (`./gradlew assembleDebug`).
2. Install: `adb install app/build/outputs/apk/debug/app-debug.apk`.
3. Open LSPosed/Vector manager, go to Modules, enable **VPN Hide**.
4. Add **"System Framework"** to the module's scope. No other apps should be in scope.
5. Reboot.
6. Open the VPN Hide app to manage target apps.

## Combined use with kmod

For apps with aggressive anti-tamper SDKs, full VPN hiding requires covering both native and Java API paths without any hooks in the target app's process:

- **[kmod](../kmod/)** covers native: `ioctl`, `getifaddrs` (netlink), `/proc/net/route`.
- **This module** covers Java APIs: `NetworkCapabilities`, `NetworkInfo`, `LinkProperties` via `writeToParcel()` in `system_server`.

Together they provide complete VPN hiding with zero footprint in the target process.

## Debugging

```bash
adb logcat | grep VpnHide
```

## Build

```bash
./gradlew assembleDebug
```

Requires JDK 17. Output: `app/build/outputs/apk/debug/app-debug.apk`.

## License

MIT. See [LICENSE](../LICENSE).
