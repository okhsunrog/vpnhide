# Persistent state - every path the project touches

Reference catalogue for answering "where is this stored?", "who reads X?",
and "what survives a reboot?". The storage source of truth is now the canonical
JSON config; native runtime channels use the control-v2 / telemetry-v1 text wire
from [protocol.md](protocol.md).

For each entry: format, writer, reader, lifetime, and permissions when relevant.

> SELinux contexts shown here are AOSP/default-root-manager expectations. Custom
> ROMs may relabel paths; if a write fails, check `dmesg | grep avc`.

---

## 1. Canonical Config

### `/data/system/vpnhide_config.json`

The single managed desired-state file.

- Format: JSON object, `version: 1`, `debug: Boolean`, `apps: { package ->
  roles }`, `settings.rememberSuperkey: Boolean`,
  `settings.optionalFeatures: [feature name]`,
  `settings.autoHideVpnServices: Boolean`, `settings.autoHideVpnName: Boolean`,
  `settings.autoHiddenPackages: [package]`.
- Roles per package: `java`, `native` (`Boolean` or hook-name array),
  `appHiding`, `ports`, and the app-owned extension `hidden`.
- Writer: VPN Hide app via `su` (`StorageConfig.kt`) on Save, startup
  self-target preparation, debug toggle, and APatch SuperKey setting.
- Readers:
  - Rust activator bins (`crates/activator`) for native and ports backends.
  - LSPosed hooks in `system_server` (`SystemServerConfigCache`) directly.
  - App UI caches (`TargetsCache`, `DashboardData`).
- Permissions: `0640 root:system`, SELinux `system_data_file`.
- Lifetime: persistent across reboot, module reinstall, and app reinstall.

The app writes it atomically via temp-file + `mv`. If it is absent, native
activators treat it as an empty config and app startup creates a new config
containing the mandatory VPN Hide self-target.

### Pre-1.0 Config Files (import inputs)

Nothing writes these; up to 0.7.1 they *were* the user's config. 1.0.0 folded
them into the canonical JSON on first launch, 1.2.0 dropped that fold (and the
cleanup with it), so a device upgrading 0.7.x → 1.2.x kept the files and lost
the settings. `LegacyConfigImport.kt` reads them again:

- `/data/adb/vpnhide_kmod/targets.txt`, `…_kpm/targets.txt`,
  `…_zygisk/targets.txt` — native role, package names.
- `/data/adb/vpnhide_lsposed/targets.txt` — Java role, package names.
- `/data/adb/vpnhide_ports/observers.txt` — ports role, package names.
- `/data/system/vpnhide_hidden_pkgs.txt` — hidden packages, package names.
- `/data/system/vpnhide_observer_uids.txt` — app-hiding role, **UIDs**, mapped
  back through the current `pm list packages -U` inventory.
- `/data/system/vpnhide_uids.txt` — derived from the LSPosed list by the old
  `service.sh`; deleted with the rest, never read back.

The import runs silently at startup when the canonical config holds no
user-configured app. When it does hold one, a Dashboard banner and a Settings →
Configuration entry offer three actions: merge, replace, or **delete without
importing** (for the user who has already reconfigured by hand and only wants
the leftovers gone — nothing else on the device removes them). All three delete
every file above and `rmdir` `/data/adb/vpnhide_zygisk` and
`/data/adb/vpnhide_lsposed`, which hold nothing else; merge and replace do it in
the same root transaction as the config write, delete runs on its own (no config
write, no activator — the running config does not change).

Absence of the files is the "already imported" marker. The banner's *Hide*
button only sets `legacyImportDismissed` in the app's `ui_settings` DataStore:
files stay, and the Settings entry keeps every action reachable.

`/data/adb/vpnhide_kmod`, `…_kpm` and `…_ports` survive: they carry live
`load_status` / `load_dmesg` / `ctl.lock`.

---

## 2. Module Install Dirs

Module dirs under `/data/adb/modules/` are replaced by Magisk/KSU/APatch on
module reinstall. They hold binaries and boot scripts, not user-managed config.

### `/data/adb/modules/vpnhide_kmod/`

- `module.prop`: module metadata, version, and stamped `gkiVariant=`.
- `post-fs-data.sh`: thin root-manager entrypoint that execs `activator boot-load`.
- `service.sh`: starts `activator boot-service` in the background and returns
  immediately to the root manager's sequential script runner.
- `uninstall.sh`: thin uninstall entrypoint that execs `activator uninstall`.
- `activator`: Rust bin that reads canonical JSON, lists Android users and
  resolves packages with `pm list packages -U --user <id>` for each user,
  loads the `.ko` during `post-fs-data`, owns its typed load diagnostics,
  waits for `/proc/vpnhide_ctl` during late-start, formats text wire, and writes
  `/proc/vpnhide_ctl`.
- `vpnhide_kmod.ko`: kernel module binary.

### `/data/adb/modules/vpnhide_kpm/`

- `module.prop`: module metadata.
- `post-fs-data.sh`: thin root-manager entrypoint that execs `activator boot-load`.
- `service.sh`: starts `activator boot-service` in the background and returns
  immediately to the root manager's sequential script runner.
- `uninstall.sh`: thin uninstall entrypoint that execs `activator uninstall`.
- `activator`: Rust bin that refuses to run when the `.ko` backend is present,
  refuses unsupported kernel families before invoking KernelPatch,
  owns load/config status and preserves the actual boot-only feature choice,
  reads optional `/data/adb/vpnhide/superkey`, loads/configures KPM through
  APatch/FolkPatch direct supercalls or KPatch-Next `kpatch kpm load` +
  `kpatch kpm ctl0` (including the standalone KPatch-Next-Module CLI path).
- `vpnhide.kpm`: KernelPatch module binary.

### `/data/adb/modules/vpnhide_zygisk/`

- `module.prop`: module metadata.
- `customize.sh`: install hook that applies module file permissions.
- `service.sh`: starts `activator boot-service` in the background and returns
  immediately to the root manager's sequential script runner.
- `uninstall.sh`: thin entrypoint that execs `activator uninstall`; canonical
  config remains app-owned.
- `activator`: Rust bin that writes the Zygisk runtime config.
- `zygisk/arm64-v8a.so`: Rust cdylib injected into app processes.
- `targets.txt`: runtime `vpnhide 2 config` text snapshot read through Zygisk's
  module-dir fd. This is not persistent user config; it is regenerated by the
  activator from canonical JSON.

### `/data/adb/modules/vpnhide_ports/`

- `module.prop`: module metadata.
- `service.sh`: starts `activator boot-service` in the background and returns
  immediately to the root manager's sequential script runner.
- `activator`: Rust bin that reads canonical JSON, derives `ports: true`
  packages, waits for netd, resolves UIDs, applies and later re-applies iptables
  rules, and records the latest apply status under `/data/adb/vpnhide_ports/`.
- `uninstall.sh`: thin entrypoint; the activator removes `vpnhide_out`,
  `vpnhide_out6`, and portshide diagnostics.

For all four module directories, the app checks the `activator` file itself in
the shared root snapshot. An enabled installation with a missing or
non-executable activator is a bundle-integrity failure even when an old runtime
status file still exists.

---

## 3. Persistent `/data/adb` State

### `/data/adb/vpnhide_kmod/`

| File | Format | Writer | Reader | Lifetime |
|---|---|---|---|---|
| `load_status` | `key=value`: timestamp, boot_id, uname_r, gki_variant, kmod_version, root_manager, kprobes, kretprobes, filesystem_hiding, filesystem_config_exit, filesystem_config_error, insmod_exit, loaded, insmod_stderr | kmod activator | app dashboard | overwritten each boot |
| `load_dmesg` | filtered dmesg excerpt | kmod activator | app dashboard/debug export | overwritten each boot |

### `/data/adb/vpnhide_kpm/`

| File | Format | Writer | Reader | Lifetime |
|---|---|---|---|---|
| `load_status` | `key=value`: timestamp, boot_id, uname_r, runtime, loaded, filesystem_hiding, reason, detail | KPM activator | app dashboard | overwritten each boot |
| `ctl.lock` | empty advisory-lock inode, mode `0600` | KPM activator | KPM activators serialize ctl0 config/status/stats calls with `flock` | while the module is installed; removed on uninstall |

### `/data/adb/vpnhide_ports/`

| File | Format | Writer | Reader | Lifetime |
|---|---|---|---|---|
| `load_status` | `key=value`: timestamp, boot_id, uname_r, runtime=ports, source, loaded, target_count, detail | ports activator | app dashboard/debug export | overwritten on every ports apply |
| `load_log` | stdout/stderr excerpt from the latest ports apply | ports activator | debug export | overwritten on every ports apply |

### `/data/adb/vpnhide/superkey`

- Format: APatch SuperKey as a plain text file with trailing newline.
- Writer: VPN Hide Settings when `settings.rememberSuperkey` is enabled.
- Reader: KPM activator.
- Permissions: `0600 root:root`.
- Lifetime: persistent, root-only, direct-encrypted storage readable at boot.

The key is intentionally not stored inside the canonical JSON because that file
is exportable and readable by `system_server`.

---

## 4. system_server Files

### `/data/system/vpnhide_lsposed_state`

- Format: protocol-shaped readback: `vpnhide 1 status` plus LSPosed `meta`
  records (`version`, `boot_id`, `timestamp`, `aosp_sdk`, optional
  `broken_fields`) and `vpnhide 1 stats` sparse counters.
- Writer: LSPosed hooks in `system_server`.
- Reader: app dashboard via root snapshot.
- Permissions: default root/system-server write behavior; app reads via `su`.
- Lifetime: per boot.

`/data/system/vpnhide_hook_active` is the retired status-only marker. The Full
Reset action removes it if it remains from an old installation.

The canonical config is also in `/data/system`, but it is covered in section 1
because it is the storage root for every layer.

---

## 5. Native Runtime Channels

### `/proc/vpnhide_ctl`

Created by the `.ko` backend at module init. Mode `0600`, root-only.

| Direction | Format | Writer/Reader |
|---|---|---|
| write | `vpnhide 2 config` text snapshot: `debug` + grouped `targets <hookmask> <uid>...` + `end` | kmod activator writes; kernel parses via shared protocol code |
| read | `status` + `stats` text response | app/debug tooling reads |

The state is in-kernel and per-boot. The proc write replaces the whole config,
so activator delivers one bounded snapshot.

### `/proc/vpnhide_diag`

Created by the `.ko` backend alongside the control node, read-only, root-only.
Diagnostic surface only: it is **not** part of the frozen control/telemetry wire
and nothing parses it. It reports what `/proc/vpnhide_ctl` cannot — per-probe
registration, kretprobe/kprobe `nmissed` counters (so an exhausted
`VPNHIDE_KRETPROBE_MAXACTIVE` is visible), the active vs installed hook masks,
and the live `is_vpn_ifname()` verdict for every netdev in the reader's netns.
The verdict is reported as-is, bugs included, so a false-positive interface match
shows up instead of being hidden by a corrected copy of the logic.

The debug bundle cats it verbatim into a `kmod_diag` section. No UI surface.

### KPM Supercall Channel

No file or proc node. The KPM activator sends the same `vpnhide 2 config` text
wire through the KernelPatch KPM ctl0 supercall. On APatch/FolkPatch it invokes
the supercall directly with the saved SuperKey when present, otherwise with the
runtime's trusted `su` token when that grant is available. On KPatch-Next it
uses `kpatch kpm ctl0` from the runtime's own CLI. Status/stats use the same
ctl0 request/response channel.

### Zygisk Module-Dir `targets.txt`

`/data/adb/modules/vpnhide_zygisk/targets.txt` is the runtime text wire consumed
through Zygisk's module-dir fd. It survives until module reinstall or the next
activator write, but it is derived state, not user storage.

---

## 6. LSPosed Runtime Resolution

The LSPosed hooks read canonical JSON directly in `system_server` and resolve
package names to appIds through `/data/system/packages.list`, never through
PackageManager. Caller matching uses `callingUid % 100000`, so all Android user
profiles for the same appId are covered. `java: true` enables every LSPosed VPN
sanitizer for that app; `java: ["hook_name", ...]` enables only the named
LSPosed Java hooks for the app. App-hiding package visibility is controlled by
`appHiding`, not by the Java VPN hook list.

Inotify watches `/data/system/vpnhide_config.json`; package reinstall UID/appId
changes are picked up by a periodic fingerprint of `/data/system/packages.list`.

---

## 7. iptables

Two chains in the filter/OUTPUT path:

- `vpnhide_out` for IPv4 loopback.
- `vpnhide_out6` for IPv6 loopback.

Writer: ports Rust activator through `iptables-restore --noflush` and
`ip6tables-restore --noflush`. The same activator writes
`/data/adb/vpnhide_ports/load_status` and `load_log` after each apply attempt.

Readers/checks: dashboard tests chain existence with `iptables -L vpnhide_out -n`.

Lifetime: in-kernel, per-boot. The ports service reapplies rules after reboot,
and the app reruns the ports activator on Save.

---

## 8. App-Process State

### SharedPreferences `vpnhide_prefs`

Accessed through `context.getSharedPreferences("vpnhide_prefs", MODE_PRIVATE)`.
Keys currently in use:

- `last_seen_version: String`: changelog dialog.
- `help_collapsed_*: Boolean`: help accordions.

`debug` / `debugSwitch` are not stored here anymore. Diagnostics and debug
capture state comes from canonical JSON (`/data/system/vpnhide_config.json`).

Vector LSPosed redirects this storage to:

```sh
/data/misc/<vector-uuid>/prefs/dev.okhsunrog.vpnhide/vpnhide_prefs.xml
```

Writing `/data/data/dev.okhsunrog.vpnhide/shared_prefs/` by hand does not affect
the app when Vector is active.

### `filesDir`

`/data/user/0/dev.okhsunrog.vpnhide/files/vpnhide_zygisk_active`

- Format: `key=value`: `version`, `boot_id`, `pid`, `timestamp`,
  `requested_hooks`, `installed_hooks`, and optional `filesystem_error`.
- Writer: Zygisk module when the current forked target process is the VPN Hide
  app. This is canonical/native self-targeting for the heartbeat, not LSPosed
  module scope.
- Reader: app root snapshot/dashboard/startup cleanup.
- Lifetime: per app launch, stale records removed when boot_id changes.

`/data/user/0/dev.okhsunrog.vpnhide/files/vhprobe`

- Format: executable Rust diagnostic probe extracted from the APK asset.
- Writer/reader: the app overwrites it on launch; root diagnostics copy it to
  `/data/local/tmp/vpnhide_vhprobe` for root-differential checks or to the
  per-process `/data/local/tmp/vpnhide_kpm_probe.<pid>` for APatch KPM listing.
- Lifetime: app-private and replaced on launch. Every `/data/local/tmp` copy is
  deleted immediately after execution.

### `cacheDir`

Used for temporary debug/export files and a copied read-only LSPosed config DB:
`vpnhide_lspd_modules_config.db` plus optional WAL/SHM sidecars. These are
deleted after use.

---

## 9. External Paths Read

| Path | Owner | Purpose |
|---|---|---|
| `/data/adb/lspd/config/modules_config.db` | LSPosed/Vector | Dashboard checks module enabled state and System Framework scope |
| `/data/misc/<vector-uuid>/prefs/<pkg>/` | Vector LSPosed | Redirected SharedPreferences |
| `/data/system/packages.list` | Android system | system_server package -> appId resolution |
| `/proc/sys/kernel/random/boot_id` | kernel | Freshness checks for per-boot records |
| `/proc/version`, `/proc/modules`, `/proc/config.gz` | kernel | kmod diagnostics |
| `/proc/net/{route,ipv6_route,if_inet6,tcp,tcp6,udp,udp6,dev,fib_trie}` | kernel | diagnostics and native hiding vectors |
| `/sys/class/net/`, `/sys/class/net/<iface>/operstate` | kernel | VPN-active detection |

---

## 10. Boot-Time Sequence

```text
post-fs-data:
  kmod post-fs-data
    -> exec activator boot-load
    -> activator reads filesystem_iface_paths from canonical JSON
    -> activator runs bounded insmod ... filesystem_hiding=0|1
    -> activator writes /data/adb/vpnhide_kmod/load_status and load_dmesg
  KPM post-fs-data
    -> exec activator boot-load
    -> activator refuses if .ko module is installed+enabled
    -> activator reads filesystem_iface_paths from canonical JSON
    -> keyless KPatch-Next: activator validates uname major.minor, then loads
       vpnhide.kpm with filesystem_hiding=1 when enabled (no load arg otherwise)
    -> APatch/FolkPatch: defer to service activator; service tries saved key,
       then trusted su token, and uses the same optional KPM load arg; without
       either credential it writes awaiting_superkey

service:
  kmod / KPM / Zygisk service.sh
    -> start that module's activator boot-service in the background and return
    -> KPM activator rejects unsupported kernels before waiting for PackageManager
    -> activator waits for PackageManager to expose dev.okhsunrog.vpnhide
    -> kmod activator also waits for /proc/vpnhide_ctl
    -> KPM activator uses saved APatch SuperKey or trusted su token when present
    -> activator reads canonical JSON and writes exactly one native channel
  ports service.sh
    -> start ports activator boot-service in the background and return
    -> activator waits for netd baseline
    -> activator waits for PackageManager readiness and applies iptables from canonical JSON
    -> activator repeats the idempotent apply after 30 seconds

system_server:
  HookEntry.handleLoadPackage
    -> install hooks
    -> write /data/system/vpnhide_lsposed_state
    -> watch canonical JSON

zygote app fork:
  zygisk on_load
    -> read module-dir targets.txt text wire
  zygisk post_app_specialize
    -> if target: install selected libc hooks; filesystem_iface_paths is
       projected only when the canonical optional feature is enabled
    -> install the optional filesystem aliases atomically; omit bit 27 from
       installed_hooks on failure without disabling the base network hooks
    -> if target process is VPN Hide app: write filesDir/vpnhide_zygisk_active
```

---

## 11. Lifetime Cheat Sheet

| Lifetime | Examples |
|---|---|
| In-kernel per boot | `/proc/vpnhide_ctl` state, KPM in-kernel state, iptables chains |
| Per boot / last apply files | `/data/adb/vpnhide_kmod/load_status`, `/data/adb/vpnhide_kmod/load_dmesg`, `/data/adb/vpnhide_kpm/load_status`, `/data/adb/vpnhide_ports/load_status`, `/data/adb/vpnhide_ports/load_log`, `/data/system/vpnhide_lsposed_state` |
| Per app launch | `filesDir/vpnhide_zygisk_active`, `filesDir/vhprobe` |
| Persistent root-managed | `/data/system/vpnhide_config.json`, `/data/adb/vpnhide/superkey` |
| Module-dir derived state | `/data/adb/modules/vpnhide_zygisk/targets.txt` |
| Removed on module uninstall | `/data/adb/vpnhide_kmod/`, `/data/adb/vpnhide_kpm/`, `/data/adb/vpnhide_ports/` when empty after deleting module-specific files |
| Removed on pre-1.0 config import (or Full Reset) | the import inputs in section 1, plus `/data/adb/vpnhide_zygisk/`, `/data/adb/vpnhide_lsposed/` |
| Wiped on module reinstall | files under `/data/adb/modules/vpnhide_*/` |
| Wiped on app reinstall | app SharedPreferences, except Vector redirects the physical path under `/data/misc/<uuid>/prefs/` |
