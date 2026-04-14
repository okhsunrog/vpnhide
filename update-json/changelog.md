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
