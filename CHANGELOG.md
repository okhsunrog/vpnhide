# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Debug logging toggle in Diagnostics: off by default — VPN Hide, LSPosed hooks (VpnHide-NC/NI/LP and the package-visibility filter), and zygisk keep logcat near-silent. Start recording and Collect debug log automatically enable verbose logging for the duration of the capture and restore it afterwards, so the toggle is only needed if you want logs emitted continuously outside a capture. Errors always pass through so hook-install failures remain visible.

### Fixed
- Dashboard now shows a consistent version string for all modules. Kernel-module, Zygisk and Ports module cards used to display the Magisk-style 'vX.Y.Z' from their module.prop, while the LSPosed hook module card showed the Android-style 'X.Y.Z' from the APK's versionName — on the same screen, for the same version number. The 'v' prefix is now stripped at parse time so every card reads 'X.Y.Z' (or 'X.Y.Z-N-gSHA' for dev builds).

## v0.6.2

### Added
- Full system logcat recording on the Diagnostics screen — Start/Stop an unfiltered capture of all logcat buffers (main/system/crash/events/radio), then Save to a user-picked location via the Storage Access Framework or Share through the system sheet. Useful for submitting diagnostic logs straight from the app without running adb logcat by hand.

### Changed
- Module zips and APK now include git provenance in the version string. Official release builds from a tag stay at the clean version (e.g. 0.6.2); intermediate builds from main or a feature branch show up as 0.6.2-5-gabc1234 (or -dirty) in the Magisk/KSU manager and Android Settings, so debug-log submissions identify the exact commit.

### Fixed
- Target apps (Ozon, Home Assistant, Megafon, Chrome, etc.) no longer hang on infinite load when vpnhide-zygisk is installed. Our recv/recvmsg hooks used to run the netlink-dump filter on every socket regardless of type — on TCP/UDP/Unix the first bytes are arbitrary user data (TLS ciphertext, HTTP body) and occasionally matched RTM_NEWLINK/RTM_NEWADDR by chance, causing the filter to mangle the buffer in-place and corrupt the TLS stream. Gated both hooks on AF_NETLINK so non-netlink traffic passes through untouched.
- Target apps no longer fail Java-level hooks on cold boot. service.sh in the kmod and zygisk modules used to resolve package names to UIDs as soon as pm list packages started responding — but PackageManager responds early with only the system-package snapshot, so user-installed targets (including the vpnhide app itself) got resolved to empty. /data/system/vpnhide_uids.txt was written with at most one UID, the LSPosed hook cached that empty set on its first writeToParcel call, and the Java-level filtering silently no-opped until the next lucky boot. Now service.sh waits until the vpnhide package itself is visible in pm list packages -U, so the full user-package index is ready before resolving.
- Ozon and other apps with root-detection scanning /proc/self/fd no longer hang when vpnhide-zygisk is installed. The module's zygote-side on_load was leaking the module-dir fd, which every forked app process inherited — root-tamper scans detected a descriptor pointing inside /data/adb/modules/ and refused to continue. The fd is now explicitly closed before any app fork.

## v0.6.1

### Added
- Detect gb.sweetlifecl as a Russian-vendor package in the app picker filter, so it shows up under the Russian-only filter alongside other domestic apps.

### Fixed
- Magisk versions before v28 failed to install vpnhide-ports and vpnhide-kmod with an unpack error — restored the META-INF/com/google/android/{update-binary,updater-script} entries the older managers expect.

## v0.6.0

### Added
- App hiding mode in Protection — hide selected apps from selected observer apps at the PackageManager level. Observer apps can no longer list, resolve, or query hidden apps.
- Ports hiding mode plus new vpnhide-ports.zip module — block selected apps from reaching 127.0.0.1 / ::1 ports to hide locally running VPN/proxy daemons (Clash, sing-box, V2Ray, Happ, etc.).
- Ports module integration in the dashboard — shows install state, active rules, observer count, and version mismatch/update warnings.

### Changed
- The old Apps tab is now Protection, split into three modes: Tun, Apps, and Ports.
- Ports rules apply immediately on Save and are restored automatically on boot.
- vpnhide-ports.zip is now included in the release/update pipeline with Magisk/KernelSU update metadata.

### Fixed
- Fixed LinkProperties filtering so VPN routes are stripped more reliably from app-visible network snapshots.
- Fixed SIOCGIFCONF filtering on some Android 12/13 5.10 kernels where the previous hook could succeed but never fire.
- Fixed debug log collection so app logcat entries are captured reliably on devices where logcat via su misses them.

## v0.5.3

### Added
- Debug log export — open the Diagnostics tab and tap "Collect debug log" at the bottom. The app gathers dmesg, check results, device info, module status, kernel symbols, targets, interfaces, routing tables, and logcat into a zip. Save to disk or share directly.
- Kernel module debug logging toggle — all 6 kretprobe hooks now log detailed info (UID, target status, interface name, filter decisions) when debug mode is active. Enabled automatically during debug log collection.

## v0.5.2

### Fixed
- Fixed SIOCGIFCONF filtering on kernel 5.10 (tun0 was visible in interface enumeration)
- Fixed zygisk first-launch race: dashboard no longer shows false "inactive" status
- Added recv hook in zygisk for netlink filtering on Android 10
- Fixed hardcoded v0.1.0 in module installer messages

## v0.5.1

### Fixed
- Fixed false "LSPosed/Vector not installed" warning when LSPosed uses non-standard module path (e.g. zygisk_lsposed)
- Fixed false LSPosed config warnings when hooks are already active at runtime
- "No target apps configured" now checks all modules, not just LSPosed

## v0.5.0

### Added
- Built-in diagnostics merged into the main app (separate test app removed)
- New app icon and chameleon mascot branding
- Dashboard with module status, LSPosed config validation, version checks, and live protection verification
- Native module install recommendation — the app detects your kernel and tells you exactly which module to download
- Per-app protection layer toggles (L/K/Z) — control LSPosed, kernel module, and Zygisk independently for each app
- Magisk/KSU auto-update for kmod and zygisk modules
- App update check via GitHub Releases
- Changelog with version history

### Changed
- Replaced WebUI with native Compose UI for module management
- Zygisk API lowered from v5 to v2 for Magisk v27 compatibility
- Apps tab: search in top bar, filter menu, fast scroll with letter indicator, Russian apps filter

### Fixed
- Fixed potential system_server crash caused by race condition in writeToParcel hooks
- App no longer removes itself from target lists when saving

## v0.4.2

### Added
- ioctl SIOCGIFMTU detection and filtering
- LinkProperties routes filtering in LSPosed hooks

### Fixed
- Fixed VPN LinkProperties deserialization crash

## v0.4.1

### Added
- Russian translation
- Root access error screen with clear instructions

## v0.4.0

### Added
- VPN Hide app with target picker UI — select apps to hide VPN from
- Built-in diagnostics — 26 checks covering all detection vectors
- Auto-detect VPN and auto-add self to target list

### Changed
- Replaced WebUI with native Compose UI

### Fixed
- Zygisk targets reading on Magisk (SELinux blocking /data/adb/)

## v0.3.1

### Changed
- CI renames APK artifacts to vpnhide-lsposed.apk and vpnhide-test.apk for clearer release downloads.

## v0.3.0

### Added
- Monorepo restructure — vpnhide-zygisk, vpnhide-lsposed, and vpnhide-kmod now live in a single repository with a unified release pipeline.
- Diagnostic test app (Compose) that exercises 23+ VPN-detection paths so you can verify the hooks are actually working on your device.
- Kernel module gains `dev_ifconf`, `inet_fill_ifaddr`, and `inet6_fill_ifaddr` kretprobes for complete native-level VPN interface hiding.
- Zygisk module now writes resolved UIDs to a shared file so the LSPosed system_server hooks can load them without reboot.

### Changed
- LSPosed module stripped down to system_server hooks only; native detection paths moved to zygisk and kmod where they belong.
- Native diagnostic checks rewritten from C++ to Rust to match the zygisk module's stack.
- Unified entire project under the MIT license.
- CI consolidated into a single workflow: parallel builds across all 7 GKI generations (Android 12/13/14/15/16 × 5.10/5.15/6.1/6.6/6.12) using the DDK container images.

### Fixed
- Removed leftover `/proc/net/*` redirect hooks from the LSPosed module — those paths are the zygisk/kmod job and were causing duplicate filtering.

## v0.2.0

### Added
- system_server-side hooks so Java API VPN hiding also works for apps using the MIR HCE SDK (which previously bypassed in-process hooks).
- Live UID reload via `FileObserver` (inotify) — adding or removing target apps no longer requires rebooting the device.

### Fixed
- `FileObserver` is now retained from GC and watches the parent directory (not the file) so edits-as-rename reliably fire the inotify event.
- UIDs are now read from `/data/system/` (readable by system_server under SELinux) and `writeToParcel` modifies the parcel in place instead of swapping the backing object.

## v0.1.0

### Added
- Initial release — LSPosed module that hides an active VPN from apps listed in the module's LSPosed scope.
- In-process Java hooks for `NetworkCapabilities`, `NetworkInfo`, `LinkProperties`, DNS servers, HTTP proxy, and VPN-related system properties.
- Hook on `NetworkCapabilities.toString()` strips VPN-identifying tokens from debug representations.
