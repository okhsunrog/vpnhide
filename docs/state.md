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
  `settings.autoHideExcludedPackages: [package]`,
  `settings.autoHiddenPackages: [package]`.
- Roles per package: `java`, `native` (`Boolean` or hook-name array),
  `appHiding`, `ports`, and the app-owned extension `hidden`.
- Writer: VPN Hide Core only (`vpnhide-core`, `crates/core-daemon`). The app
  submits edits over the core socket (typed field batches / explicit repair) on Save,
  startup self-target preparation, the debug toggle and the APatch SuperKey
  setting; it runs no root process and no `su`.
- Readers:
  - Core, which projects it onto the selected native backend and Ports (the
    1.x per-backend activator binaries are gone; `crates/activator` is now a
    library core links).
  - LSPosed hooks in `system_server` (`SystemServerConfigCache`) directly.
  - App UI caches (`TargetsCache`, `DashboardData`), through core's published state.
- Permissions: `0640 root:system`, SELinux `system_data_file`.
- Lifetime: persistent across reboot, module reinstall, and app reinstall.

The process-owned `CanonicalConfigRepository` orders all app writes through core
operation receipts and publishes typed confirmed state separately from observation
caches. Core writes the file atomically (temp file + rename + directory fsync) and journals the
operation. If it is absent, core treats it as an empty config and app startup
creates a new config containing the mandatory VPN Hide self-target.

### Core socket and operation journal

- `/data/user_de/0/dev.okhsunrog.vpnhide/vpnhide-core.sock`: core's tarpc socket,
  bound inside the app's device-encrypted data dir with that dir's owner and
  SELinux label, peer-checked with `SO_PEERCRED`. Only the release package in user
  0 has it; the `.dev` sandbox build and secondary-user installs cannot reach core.
- `/data/adb/vpnhide/core/operations.json`: core's operation journal (receipts the
  app re-queries after an epoch change). See section 3.
- `/data/system/vpnhide_config.json.lock` and
  `/data/adb/vpnhide/core/operations.json.lock`: permanent root-only regular
  lockfiles. Core holds both flocks from before load until shutdown. Alternate
  state paths cannot bypass ownership of either resource. FullReset preserves them.
- `<app data dir>/vpnhide-core.sock.lock`: root-owned socket lifetime lock,
  preventing another server from unlinking a live socket. The app need not open it.

The 1.x root mutation transport (`/data/adb/vpnhide/app-state/`, the staged
`vhhelper-<sha256>` executable and its lane lock) is gone; Full Reset removes a
leftover `app-state/` with the rest of `/data/adb/vpnhide/` except `core/`.

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
write, no backend apply — the running config does not change).

Absence of the files is the "already imported" marker. The banner's *Hide*
button only sets `legacyImportDismissed` in the app's `ui_settings` DataStore:
files stay, and the Settings entry keeps every action reachable.

`/data/adb/vpnhide_kpm` survives: it carries the live `ctl.lock`.

---

## 2. Module Install Dirs

Module dirs under `/data/adb/modules/` are replaced by Magisk/KSU/APatch on
module reinstall. They hold artifacts, not user-managed config. **No shell logic
runs on the device** at boot: `vpnhide-core` (in `vpnhide_core`) is the only
loader and writer, its `service.sh` is a single `exec`, and the backend modules
ship no script at all (the core zip's install-time `customize.sh` only moves the
ABI binary into place). See
[notes/core-2.0-module-lifecycle.md](notes/core-2.0-module-lifecycle.md).

| Module | Contents |
|---|---|
| `vpnhide_core` | `module.prop`, `vpnhide-core` (device ABI, kept by the 3-line `customize.sh`), `service.sh` (`exec "${0%/*}/vpnhide-core" service`), `sepolicy.rule` |
| `vpnhide_kmod` | `module.prop` (stamped `gkiVariant=`), `vpnhide_kmod.ko` — core loads it with `finit_module(2)` |
| `vpnhide_kpm` | `module.prop`, `vpnhide.kpm` — core loads it through KernelPatch |
| `vpnhide_zygisk` | `module.prop`, `zygisk/<abi>.so`, `targets.txt` (runtime `vpnhide 2 config` snapshot core writes; not user config) |

The in-tree (`CONFIG_VPNHIDE=y`) backend has no module: core adopts it when
`/proc/vpnhide_ctl` reports backend `0x4`.

---

## 3. Persistent `/data/adb` State

### `/data/adb/vpnhide/core/`

Mode `0700`, root-only. Journal, ownership markers, `core.log`, plus:

| File | Format | Writer | Reader | Lifetime |
|---|---|---|---|---|
| `operations.json` | journal schema 2: issued-ID counter, expiry floor, retained receipts, actual revision/write-set evidence, held resources, expiring debug leases and pending logging restoration operation | core writer queue, atomically with directory fsync | startup recovery and receipt queries; schema 1 is migrated, corrupt/unsupported state refuses mutation | preserved by FullReset; 64 terminal receipts plus active/restoration records; expired Unknown receipts retain compact held-resource/Persist evidence |
| `operations.json.lock` | permanent empty regular inode, mode `0600`, exclusive flock | core lifetime owner | another core refuses ownership before load/boot/bind | preserved by FullReset; never unlink a live lockfile |
| `supervisor.lock` | permanent root-only regular inode and exclusive flock | service supervisor | duplicate service invocation refuses | supervisor lifetime; inode persists |
| `supervisor_status` | running/restarting/stopped status and retry delay | service supervisor | diagnostics/manual inspection | overwritten as worker lifecycle changes |
| `native_boot.json` | serde JSON of `core-rpc` `NativeBootStatus`, keyed by `boot_id` | core, once per boot (updated by deliveries and Ports applies) | core (re-adopts on restart in the same boot); debug export. The app reads the same status over RPC (`backend.boot`). | replaced on the next boot's scan |
| `boot_status` | `key=value` boot pass report | core boot passes | debug export | overwritten each boot |
| `boot_phase` | `boot_phase=service` | `vpnhide-core service` | debug export | overwritten each boot |
| `logcat/session-<id>.log` | raw `logcat -b all -v threadtime` (0600) | core's logcat session child, through core (capped at 64 MiB / 30 min) | core, which copies it to the app's export dir on Stop | kept until delivered: deleted on the app's ack, on discard, and by the next Start; kept across a core restart while undelivered |
| `logcat/session.json` | serde JSON session record (0600): id, start, limits, end state, debug status and `delivered` | core, atomically with directory fsync | core at start (`recover`: end an unfinished session as `core_restarted`, release its generic lease, keep an undelivered recording) | latest session only; discard/next Start/delivered cleanup may delete it; the independent restoration obligation stays in `operations.json` |

The per-backend `load_status` / `load_dmesg` files (`/data/adb/vpnhide_kmod/`,
`…_builtin/`, `…_kpm/`) are gone: a kmod load failure's insmod exit, stderr and
matching kernel log lines live in `NativeBootOutcome::LoadFailed`.

### `/data/adb/vpnhide_kpm/`

| File | Format | Writer | Reader | Lifetime |
|---|---|---|---|---|
| `ctl.lock` | empty advisory-lock inode, mode `0600` | core | core serializes ctl0 config/status/stats calls with `flock` | while the module is installed |

### `/data/adb/vpnhide/superkey`

- Format: APatch SuperKey as a plain text file with trailing newline.
- Writer: VPN Hide Settings when `settings.rememberSuperkey` is enabled.
- Reader: core (KernelPatch authentication for the KPM).
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
- Permissions: default root/system-server write behavior; the app reads it through core's root snapshot.
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
| write | `vpnhide 2 config` text snapshot: `debug` + grouped `targets <hookmask> <uid>...` + `end` | core writes; kernel parses via shared protocol code |
| read | `status` + `stats` text response | app/debug tooling reads |

The state is in-kernel and per-boot. The proc write replaces the whole config,
so core delivers one bounded snapshot.

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

No file or proc node. Core sends the same `vpnhide 2 config` text
wire through the KernelPatch KPM ctl0 supercall. On APatch/FolkPatch it invokes
the supercall directly with the saved SuperKey when present, otherwise with the
runtime's trusted `su` token when that grant is available. On KPatch-Next it
uses `kpatch kpm ctl0` from the runtime's own CLI. Status/stats use the same
ctl0 request/response channel.

### Zygisk Module-Dir `targets.txt`

`/data/adb/modules/vpnhide_zygisk/targets.txt` is the runtime text wire consumed
through Zygisk's module-dir fd. It survives until module reinstall or the next
core write, but it is derived state, not user storage.

---

## 6. LSPosed Runtime Resolution

The LSPosed hooks read canonical JSON directly in `system_server` and resolve
package names to appIds through `/data/system/packages.list`, never through
PackageManager. Caller matching uses `callingUid % 100000`, so all Android user
profiles for the same appId are covered. `java: true` enables every LSPosed VPN
sanitizer for that app; `java: ["hook_name", ...]` enables only the named
LSPosed Java hooks for the app. App-hiding package visibility is controlled by
`appHiding`, not by the Java VPN hook list.

Packages sharing an appId use the **union** of their enabled Java hook lists.
A disabled Java role (or empty list) contributes no hooks; it does not veto
another package's selection. Binder cannot distinguish those packages at the
UID boundary, so the union applies to the whole group in every profile. Canonical
JSON still stores each package's intent. The Java hook dialog explains this when
the package inventory identifies a shared appId; the offline guide describes it
regardless of inventory availability.

Inotify invalidates the cache on canonical-config changes. Independently, a
one-second poll fingerprints **both** canonical JSON and `packages.list`, including
device/inode, size and nanosecond modification/change times. A load compares both
fingerprints before and after reading, and publishes only under the same
invalidation generation. A late invalidation cannot be overwritten by a reader.
Each call attempts at most two reads. If the sources keep changing (or cannot be
statted), it returns the previous coherent snapshot, or an empty fail-open policy
at startup. A separate one-second retry deadline bounds I/O and error reporting,
even if more invalidations arrive; it does not mark the old snapshot as current.
The first call at or after that deadline retries without needing a watcher event.
A stable missing canonical file or explicit empty config replaces the previous policy. Read/parse errors retain the last successful policy (or fail open at startup)
and trigger another read after one second even if metadata did not change. A
blank, malformed or unsupported JSON file is an error; intentional reset is a
valid empty canonical object or removal of the canonical file. LSPosed telemetry
exports `policy_read_status=error` and `policy_read_error` until reading succeeds.
Telemetry uses same-directory atomic rename only; replacement failure preserves
the previous complete file and schedules another write without clearing counters.

---

## 7. iptables

Two chains in the filter/OUTPUT path:

- `vpnhide_out` for IPv4 loopback.
- `vpnhide_out6` for IPv6 loopback.

Writer: core through `iptables-restore --noflush` and
`ip6tables-restore --noflush`. Each apply's per-family outcome is published as
`NativeBootStatus.ports` (and in the operation record for edits).

Readers/checks: core's root snapshot probe tests chain existence (`ports_chain`).

Lifetime: in-kernel, per-boot. Core reapplies rules after reboot (and again at
T+30 s), and on every Save.

---

## 8. App-Process State

### App UI language

Settings → Appearance → Language uses AndroidX AppCompat's per-app locales.
An empty locale list follows the system; explicit choices are `en`, `ru`, and
`zh-CN`. On Android 13+ the platform owns persistence and exposes the same
choice in system app-language settings through `res/xml/locales_config.xml`.
On Android 9–12 AppCompat's `autoStoreLocales` stores the choice in app-private
storage. This preference never enters the root canonical config or core.
Activity recreation preserves saveable navigation state. Locale changes update
Dashboard wording and diagnostic labels without invalidating root observations
or requesting another diagnostic run.

### DataStore `ui_settings`

`settings/SettingsRepository.kt` keeps UI preferences other than language in one Preferences
DataStore named `ui_settings` (`filesDir/datastore/ui_settings.preferences_pb`):
theme (`dynamic_color`, `amoled`, `contrast`, `seed_color`, `corner_style`,
`theme_mode`), `animations_enabled`, `haptics_enabled`,
`full_protection_role_labels`, `background_update_checks`,
`agent_control_enabled`, the developer switches `suppress_changelog` and
`ignore_version_mismatch` (both still fall back to `suppress_version_warnings`,
the single key they were split out of, until each is set on its own),
`settings_hint_seen`,
`donate_prompt_dismissed`, `legacy_import_dismissed`, and the update worker's
`last_notified_update_version`. Nothing here affects hiding: the runtime flags
(`debug`, `debugSwitch`) and every role live in the canonical JSON. It survives
reboots and app updates and is wiped on app reinstall; Vector's prefs redirect
does not apply to it.

### SharedPreferences `vpnhide_prefs`

Accessed through `context.getSharedPreferences("vpnhide_prefs", MODE_PRIVATE)`.
Keys currently in use:

- `last_seen_version: String`: changelog dialog.
- `help_collapsed_*: Boolean`: help accordions.

`debug` / `debugSwitch` are not stored here anymore. Diagnostics and debug
capture state comes from core's typed publication, durable leases and recorder
session. Canonical JSON holds the effective runtime logging flags.

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

Root-differential checks (ground truth, per-app VPN state) run inside core
(`collect_diagnostics` probes `GroundTruthChecks`, `AppVpnState`); a live KPM is
identified by core's boot selection, not by a KPM listing. The 1.x `vhhelper`
staged into `/data/local/tmp` is gone.

### `cacheDir`

Used for temporary debug/export files (core writes oversized probe outputs,
kernel images and the finished `logcat-session-<id>.txt` into the app's export dir,
owned by the app) and a copied read-only
LSPosed config DB:
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
service (late start):
  vpnhide_core service.sh
    -> exec vpnhide-core service (persist dir, boot_phase, fork + setsid)
    -> load journal, fence/reconcile
    -> native selection, once per boot_id (native_boot.json):
       candidates = installed kmod/KPM/Zygisk modules (no disable/remove)
                    + live built-in (ctl 0x4) + a live KPM core did not load
       >1 candidate -> Conflict: load nothing, deliver nothing, retire Zygisk targets
       1 candidate  -> load (kmod finit_module, KPM via KernelPatch) or adopt
       0            -> NoBackend
    -> boot passes thread: wait for PackageManager, deliver config to the
       selection only, wait for netd bw_OUTPUT (<= 60 s), apply Ports,
       re-apply Ports at T+30 s
    -> serve the app socket

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
| Per boot / last apply files | `/data/adb/vpnhide/core/native_boot.json`, `/data/adb/vpnhide/core/boot_status`, `/data/system/vpnhide_lsposed_state` |
| Per app launch | `filesDir/vpnhide_zygisk_active` |
| Persistent root-managed | `/data/system/vpnhide_config.json`, `/data/adb/vpnhide/superkey` |
| Module-dir derived state | `/data/adb/modules/vpnhide_zygisk/targets.txt` |
| Cleaned by core's next boot scan | a removed backend module's leftover state (e.g. `/data/adb/vpnhide_kpm/`) |
| Removed on pre-1.0 config import (or Full Reset) | the import inputs in section 1, plus `/data/adb/vpnhide_zygisk/`, `/data/adb/vpnhide_lsposed/` |
| Wiped on module reinstall | files under `/data/adb/modules/vpnhide_*/` |
| Wiped on app reinstall | app SharedPreferences (except Vector redirects the physical path under `/data/misc/<uuid>/prefs/`) and the `ui_settings` DataStore |
