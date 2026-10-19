# core-2.0 IPC authorization (step 7)

## Gates

1. **DAC** on the path socket (`/data/adb/vpnhide/core/vpnhide-core.sock`): mode `0660` when
   `VPNHIDE_CORE_SOCKET_OWNER_UID` is set (first allowlisted uid at boot), else `0666` for
   host/dev. Parent dir is `0700`.
2. **SELinux** (`core/module/sepolicy.rule`): `connectto` for `untrusted_app_all` onto
   magisk/ksu sockets; `dir`/`sock_file` search+write on common labels.
3. **SO_PEERCRED** uid allowlist (fail-closed when configured):
   - Env `VPNHIDE_CORE_ALLOW_UIDS` (comma-separated), or
   - File `VPNHIDE_CORE_ALLOW_UIDS_FILE` (default `/data/adb/vpnhide/core/allowed_uids`)
   - Empty/missing → **Open** (dev/host). Production app writes its uid after one-shot su
     (`CoreAllowlist.writeSelfUidScript`). Reinstall/UID change: rewrite file + restart core.

Package/signature mapping via `packages.xml` is a follow-up; uid allowlist is portable across
Magisk / KernelSU / APatch today.

## Resource bounds

| Bound | Value |
|-------|--------|
| Max frame | `MAX_FRAME_BYTES` = 1 MiB (`LengthDelimitedCodec`) |
| Concurrent connections | `MAX_CONCURRENT_CONNECTIONS` = 8 |
| State long-poll | `STATE_HOLD` / `STATE_POLL_DEADLINE` (existing) |

## APK path

- UniFFI `CoreClientHandle`: `hello`, `submit_edit_json`, `get_operation_json`, state mirror.
- Kotlin `CoreFfiGateway` (dev source set): reconnect on transport failure / epoch change;
  operation receipts queried independently of state updates.
- `CoreIpcPrefs.useCoreReplica` default **false** — legacy `ConfigRootIo` / root-snapshot
  remain authoritative until device validation. Editor drafts stay in Kotlin.
- Parser `CoreReplicaParser` honors freshness: last-good is display-only when `Unknown`.

## Not in this step

- Full diagnostics join (step 8)
- Wholesale activator deletion (step 9)
