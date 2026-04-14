## v0.5.3

### Added
- Debug log export on the Diagnostics screen — tap "Collect debug log" at the bottom of the Diagnostics tab to gather detailed diagnostic data (dmesg, check results, device info, module status, kernel symbols, targets, network interfaces, routing tables, logcat). After collection, save the zip to disk or share it directly. Useful for reporting issues.
- Kernel module debug logging toggle (`/proc/vpnhide_debug`) — when enabled, all 6 kretprobe hooks log detailed information (UID, target status, interface name, filter decisions). The app enables it automatically during debug log collection and disables it afterwards.

## v0.5.2

### Fixes
- Fixed SIOCGIFCONF filtering on kernel 5.10 (tun0 was visible in interface enumeration)
- Fixed zygisk first-launch race: dashboard no longer shows false "inactive" status
- Added recv hook in zygisk for netlink filtering on Android 10 (no SELinux block)
- Diagnostic checks now use recvmsg (matching the hooked path) with separate recv check
- Fixed hardcoded v0.1.0 in module installer messages

## v0.5.1

### Fixes
- Fixed false "LSPosed/Vector not installed" warning for non-standard module paths (e.g. zygisk_lsposed)
- Fixed spurious LSPosed config warnings when hooks are already active at runtime
- "No target apps configured" now checks all modules, not just LSPosed
- Renamed APK artifact to vpnhide.apk

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
