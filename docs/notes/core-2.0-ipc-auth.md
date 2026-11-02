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

Plain runs (no `VPNHIDE_APP_DATA_DIR`) default to the effective UID and socket mode
`0600`. `VPNHIDE_CORE_ALLOW_UIDS` accepts a nonempty comma/whitespace-separated list
of decimal UIDs; malformed, overflowing or non-UTF-8 values fail startup before
opening durable state. Root is not implicitly added to an explicit UID list.
`VPNHIDE_CORE_SOCKET_OWNER_UID` and octal `VPNHIDE_CORE_SOCKET_MODE` are validated
plain-socket overrides. AppOwner takes precedence over these plain-only settings
and derives placement, ownership and mode together in `EndpointConfig`.
There is no environment-selected Open policy; explicit Open library callers exist
for tests. This is IPC configuration, not a filesystem or process sandbox.

`service` and `service-worker` replace inherited `VPNHIDE_*` settings with compiled
production values before starting threads; PATH and root-manager variables remain.
Process integration tests instead clear inherited environment and explicitly supply
fixture paths and flags. Crash/delay injection is compiled out on Android; host
integration binaries retain it (they are built without `cfg(test)`).

Binding acquires endpoint ownership, binds, completes mandatory DAC/label preparation,
and only then starts accepting. Failed preparation closes/unlinks the endpoint and
releases its lock. The app-socket supervisor retries without restarting core or
replaying boot effects. An already-correct SELinux label needs no relabel syscall.
Known missing xattr support is allowed on hosts or Android without mounted SELinux;
with Android SELinux present, a missing/wrong label must be repaired and verified.
Other read/write errors propagate. Peer authorization remains required in every case.

## Resource bounds

| Bound | Value |
|-------|--------|
| Max frame | `MAX_FRAME_BYTES` = 1 MiB (`LengthDelimitedCodec`) |
| Resulting canonical config | 512 KiB each for disk and normalized RPC JSON (`config_edit::validate_candidate`; app additionally checks disk size) |
| Concurrent connections | `MAX_CONCURRENT_CONNECTIONS` = 8 |
| State long-poll | `STATE_HOLD` / `STATE_POLL_DEADLINE` |
| Writer queue | 16 tasks; overload returns Busy |
| RPC requests / driver futures | 16 / 32 per connection; 128 / 256 globally at 8 connections |
| Physical RPC blocking workers | 6 ordinary + 2 recovery + 2 recording-status; permits survive waiter cancellation |
| Held state polls | 2 per connection; extra polls return current publication |
| RPC writer queue wait | 2 seconds before start; expired/cancelled commands cannot execute later |
| Diagnostic collections | 2 forensic + 1 reserved standalone RootSnapshot; at most 32 probes and 4 workers each |

## APK connection and save path (protocol 15)

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

Host tests cover label decision/error cases, preparation ordering, endpoint retries,
app-directory disappearance and dynamic owner lookup. They do not verify Android
SELinux policy. Real label copying, `connectto` under Magisk/KernelSU/APatch, first
install, clear-data and reinstall with a changed UID still require device acceptance.

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
