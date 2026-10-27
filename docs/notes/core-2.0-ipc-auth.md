# core-2.0 IPC authorization

The app runs no root process. It reaches core with the in-process UniFFI client
(`crates/core-ffi`, loaded through JNA) over a path socket that core binds **inside the app's
own device-encrypted data dir**. Core is the only root process and the only writer of the
canonical config, `/proc/vpnhide_ctl`, Zygisk `targets.txt` and the Ports rules.

## Socket placement (`crates/core-daemon/src/app_socket.rs`)

- Path: `/data/user_de/0/dev.okhsunrog.vpnhide/vpnhide-core.sock`
  (core-daemon `app_socket`, from `VPNHIDE_APP_DATA_DIR` set by the Rust service entry; the app
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
| Resulting canonical config | 512 KiB (`config_edit::validate_candidate`; app: `MAX_CORE_CONFIG_BYTES`) |
| Concurrent connections | `MAX_CONCURRENT_CONNECTIONS` = 8 |
| State long-poll | `STATE_HOLD` / `STATE_POLL_DEADLINE` |
| Writer queue | 16 tasks; overload returns Busy |
| Heavy diagnostic collections | 2; each has at most 32 probes and 4 workers |

## APK connection and save path (protocol 11)

- One process-retained UniFFI `CoreSession` owns a shared multiplexed connection.
  `awaitState(epoch, revision)` long-polls core publications; reads and commands use
  the same handle. Every newly connected transport passes the protocol handshake.
- `core-client` owns transport generation without a watch-state mirror. Kotlin's
  single reducer owns current/last-good publication, fencing generations, request
  success/failure order, epochs and revisions.
  Disconnect withdraws current evidence; reconnection retries reads, not writes.
- The app intent coordinator obtains a durable core-issued operation ID and
  submits one typed `CoreCommand` with a base config revision. Ordinary edits
  contain one `Batch`. Import/bootstrap uses `Replace`;
  explicit repair uses `RepairActivation`. Core plans all Persist/Native/Ports
  phases; the APK has no root phase executor or `CoreConfigIo` adapter.
- A lost submission is queried by the same operation ID. SuperKey changes,
  import plus cleanup and FullReset use the same durable compound-command model.
  No uncertain command is replayed automatically.
- Editor drafts, UI preferences and app-context measurements remain in Kotlin.

See [current core architecture](../core-architecture.md) and
[app configuration intents](../config-coordinator.md).

## Device validation still required

Socket label copy, `connectto` under each root manager, and MLS on `connectto` are verified on
host only (label copy is best-effort there).

## Root work: typed probes and effects

The app runs no root process. Everything that used to need `su` is a typed RPC on the
same peercred-authenticated socket:

- `collect_diagnostics(DiagnosticsRequest { probes, clear_dmesg })` runs a fixed probe
  set (`DiagnosticProbe`: root/debug snapshots, hook counters, dmesg, logcat by app tags /
  LSPosed boot / since a validated timestamp, kernel partitions and images, the LSPosed
  modules db, ground-truth checks, per-uid VPN state). Probes are Rust in
  core (`crates/core-daemon/src/device_probes`, run as core's own bounded
  `vpnhide-core probe …` child; no shell); there is no way to pass a command. The kernel log is cleared (`klogctl`) only when `clear_dmesg` is set. Output up to 64 KiB is inline;
  larger output and artifacts are written to `<app data dir>/vpnhide-core-export/`, owned
  by the app uid (0600/0700) with the dir's SELinux label, and the report carries paths.
- `submit_command(op, CoreCommand)` admits edits, `SetSuperkey`, `ClearSuperkey`,
  import with optional legacy cleanup, cleanup and FullReset. Privileged effect
  phases require the service's `VPNHIDE_ROOT_EFFECTS=1`. Credentials are never
  journaled; the journal stores non-secret metadata and phase outcomes.
- Generic expiring debug leases are core-owned and share durable logging
  restoration obligations. Logcat has a separate durable session lifecycle.

All these APIs cross the shared UniFFI session as generated records, enums and
errors. There are no hand-written JSON control codecs or manual FFI entry points.
Probe content and import/export files may still contain JSON. Non-root probes
remain in the real app process and run alongside the core request.
