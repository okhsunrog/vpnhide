# core-2.0 IPC authorization

The app runs no root process. It reaches core with the in-process UniFFI client
(`crates/core-ffi`, loaded through JNA) over a path socket that core binds **inside the app's
own device-encrypted data dir**. Core is the only root process and the only writer of the
canonical config, `/proc/vpnhide_ctl`, Zygisk `targets.txt` and the Ports rules.

## Socket placement (`crates/core-daemon/src/app_socket.rs`)

- Path: `/data/user_de/0/dev.okhsunrog.vpnhide/vpnhide-core.sock`
  (core-daemon `app_socket`, from `VPNHIDE_APP_DATA_DIR` set by `service.sh`; the app
  resolves the same file through `CorePaths.socket`).
  Device-encrypted storage exists at boot, before first unlock. `/data/adb` is `0700` root and
  unreachable from the app.
- Stealth: another app cannot traverse a foreign app's private data dir, so there is nothing to
  probe — no world-visible socket, no abstract name.
- Supervisor: core re-binds whenever the dir exists and the socket is missing (first install,
  reinstall, "clear data"). No restart needed.

## Gates

1. **DAC**: socket owner = the dir's uid/gid, mode `0600`.
2. **SELinux**: core copies the dir's label (`app_data_file` + the app's MLS categories) onto the
   socket, so the inode is ordinary app data. `core/module/sepolicy.rule` grants only
   `connectto` from `untrusted_app_all` onto the root manager's domain (`magisk`, `ksu`, `su`).
3. **SO_PEERCRED**: `Allowlist::AppOwner` admits root and the uid that owns the app data dir,
   re-resolved on every accept (a reinstall's new uid is followed automatically). Nothing is
   written by the app; there is no uid file.

Host/dev runs (no `VPNHIDE_APP_DATA_DIR`) keep the plain path socket with the env allowlist
(`VPNHIDE_CORE_ALLOW_UIDS`, open when unset).

## Resource bounds

| Bound | Value |
|-------|--------|
| Max frame | `MAX_FRAME_BYTES` = 1 MiB (`LengthDelimitedCodec`) |
| Canonical config on the save path | `config_relay::MAX_CONFIG` = 512 KiB (app: `MAX_CORE_CONFIG_BYTES`) |
| Concurrent connections | `MAX_CONCURRENT_CONNECTIONS` = 8 |
| State long-poll | `STATE_HOLD` / `STATE_POLL_DEADLINE` |

## APK save path

- core-ffi `config_state` / `config_replace` / `config_repair` / `config_operation`: one JSON
  reply each (`status` + receipt + state); the config crosses as on-disk canonical JSON through
  core's codec (`config_file`). Kotlin bindings are generated at build time
  (`generateCoreFfiBindings`).
- Kotlin `FfiCoreConfigClient` → `CoreConfigIo` (the coordinator's effect adapter): Persist is
  `Replace` against the freshly read revision; Native/Ports report core's receipt, or submit
  `RepairActivation` when no receipt is current. An uncertain submit is recovered by querying
  the same op id, never resubmitted.
- Editor drafts and local preferences stay in Kotlin.

## Device validation still required

Socket label copy, `connectto` under each root manager, and MLS on `connectto` are verified on
host only (label copy is best-effort there).

## Root work: typed probes and effects (protocol 5)

The app runs no root process. Everything that used to need `su` is a typed RPC on the
same peercred-authenticated socket:

- `collect_diagnostics(DiagnosticsRequest { probes, clear_dmesg })` runs a fixed probe
  set (`DiagnosticProbe`: root/debug snapshots, hook counters, dmesg, logcat by app tags /
  LSPosed boot / since a validated timestamp, kernel partitions and images, the LSPosed
  modules db, ground-truth checks, per-uid VPN state, KPM list). Probes are Rust in
  core (`crates/core-daemon/src/device_probes`, run as core's own bounded
  `vpnhide-core probe …` child; no shell); there is no way to pass a command. The kernel log is cleared (`klogctl`) only when `clear_dmesg` is set. Output up to 64 KiB is inline;
  larger output and artifacts are written to `<app data dir>/vpnhide-core-export/`, owned
  by the app uid (0600/0700) with the dir's SELinux label, and the report carries paths.
- `root_effect(op, RootEffect)` applies a fixed effect: `SetSuperkey`, `ClearSuperkey`,
  `DeleteLegacyConfig`, `FullReset` (also drops core's config and publishes the new
  revision). Enabled only when the service sets `VPNHIDE_ROOT_EFFECTS=1`.

The app reaches both through core-ffi (`collectDiagnostics`, `rootEffect`; JSON in, one
JSON reply with a `status` out). Non-root probes of what an unprivileged app sees stay in
the app and run alongside the core request.
