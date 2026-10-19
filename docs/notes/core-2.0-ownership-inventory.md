# Core-2.0 exclusive ownership inventory (step 4)

Gate before production writes: **one owner per migrated resource** across install,
upgrade, interruption, and rollback. Dev paths stay explicitly enabled while legacy
writers still exist (`VPNHIDE_*` overrides, marker opt-in). Production default-on and
Magisk packaging are **not** flipped here (steps 5 / 9).

Cutover order per boundary (§11): install core → core **adopts** existing state →
legacy writer **fenced** → only then core’s first write.

Backend handoff: publishing a new Zygisk `targets.txt` does **not** clear policy
already loaded in running processes; stopping an activator does **not** unload kernel
hooks. Markers fence **writers**, they do not roll back prior applies.

---

## 1. Durable canonical config

| | |
|--|--|
| **Path** | `/data/system/vpnhide_config.json` |
| **Format** | Frozen JSON `version: 1` (LSPosed reads directly; schema must not drift) |
| **Writers (legacy)** | APK via `su` (`StorageConfig.kt` / `CanonicalConfigRepository`); full reset; debug / self-target preparation |
| **Writers (core)** | `config_file::write_atomic` on Persist |
| **Readers** | Core (restart adopt), LSPosed (`SystemServerConfigCache`), activators (projection input), app snapshot |
| **Adopt** | `ownership::adopt_config_on_start` when `VPNHIDE_CORE_ADOPT=1` or `VPNHIDE_CORE_OWNER_DIR` set: `ConfigLoad::{Absent,Present,UnsupportedVersion}`; claim `config_owned` only for Absent/Present |
| **Fence** | Marker `/data/adb/vpnhide/core/config_owned`. Core claims before Persist (best-effort). **APK still writes today** — must check marker before production cutover (not yet) |
| **Invalid ≠ absent** | `read_status`: Absent (missing/blank); Present; UnsupportedVersion; `Err` for corrupt/unreadable. Unsupported never becomes empty defaults |

---

## 2. Private operation journal

| | |
|--|--|
| **Path** | `/data/adb/vpnhide/core/operations.json` (env `VPNHIDE_JOURNAL_PATH`) |
| **Writer** | Core only |
| **Adopt / fence** | Already private; no legacy writer. Restart fences Running→Unknown; reconcile without replay |

---

## 3. Native kernel activation (kmod / built-in)

| | |
|--|--|
| **Channel** | `/proc/vpnhide_ctl` (control-v2 / telemetry-v1) |
| **Writers (legacy)** | `activator` bins: `activate_kmod` / `activate_builtin`, `boot-load`, `boot-service`; APK `AgentControl` su paths |
| **Writers (core)** | `activation::deliver_selected` when `VPNHIDE_KMOD_CTL` set |
| **Module scripts** | `kmod/module/{post-fs-data,service,uninstall}.sh`, `builtin/module/{…}.sh` → exec activator |
| **Adopt** | Core reads live backend id; does not invent apply success from telemetry alone |
| **Fence** | Marker `native_owned`. Core claims before Native deliver. Legacy `activate_*` / `uninstall_*` no-op when present |
| **Handoff** | Fence does not unload `.ko` / built-in hooks |

---

## 4. KPM activation

| | |
|--|--|
| **Channel** | APatch KPM control + lock file |
| **Writers (legacy)** | `activator` kpm bin: activate / boot-load / boot-service / uninstall |
| **Writers (core)** | Same Native phase when selector chooses KPM |
| **Fence** | Shares **`native_owned`** (mutually exclusive natives). Uninstall no-ops when owned |
| **Handoff** | Stopping activator does not unload an already-loaded KPM |

---

## 5. Zygisk publication

| | |
|--|--|
| **Path** | `$MODDIR/targets.txt` under Zygisk module |
| **Writers (legacy)** | `activate_zygisk` / boot-service / uninstall |
| **Writers (core)** | `zygisk::publish` via Native phase |
| **Fence** | Marker `zygisk_owned`. Activate/uninstall no-op when present |
| **Handoff** | New file does not revoke hooks already in running processes |

---

## 6. Ports firewall

| | |
|--|--|
| **Chains** | `vpnhide_out` / `vpnhide_out6` + OUTPUT jumps |
| **Writers (legacy)** | `portshide` activator boot-service / uninstall |
| **Writers (core)** | Ports phase when `VPNHIDE_PORTS=1` |
| **Fence** | Marker `ports_owned` (step 3). Activate recorded + uninstall teardown no-op when owned |
| **Restart** | Inspect surviving chains; do not assume daemon death cleared rules |
| **Partial** | IPv4/IPv6 reported independently on `OperationRecord.ports` |

---

## 7. Status / diagnostics side files

| Path | Legacy writer | Notes |
|------|---------------|--------|
| `/data/adb/vpnhide_ports/load_status` | Ports activator / core | Not canonical; diagnostic only |
| Kmod/KPM/builtin load_status under module dirs | Activators | Fenced with native ownership for uninstall |
| `/data/adb/vpnhide_lsposed_state` etc. | App / hooks | Out of Ports/Native migrate set; inventory in `docs/state.md` |

---

## 8. APK / CLI entry points (still live)

| Entry | What it does | Step 4 status |
|-------|----------------|---------------|
| `CanonicalConfigRepository` + `StorageConfig` | Writes canonical JSON via su | **Not fenced in APK yet** — inventory + marker ready; app must honor `config_owned` before exclusive cutover |
| `AgentControl` activation | Invokes per-backend activators | Activator-side fence when markers set |
| `app-helper` CLI | Isolated test paths | Host isolation via env; no production default |
| Full reset | Deletes config + side files | Must learn markers before production exclusive mode |

---

## 9. Marker directory

```
/data/adb/vpnhide/core/
  config_owned
  native_owned
  zygisk_owned
  ports_owned
  operations.json   # journal (separate concern)
```

Env (dev / tests):

| Variable | Role |
|----------|------|
| `VPNHIDE_CORE_OWNER_DIR` | Relocate marker dir; also enables adopt-on-start |
| `VPNHIDE_CORE_ADOPT` | Force adopt+claim config on core load |
| `VPNHIDE_PORTS_OWNER_MARKER` | Ports marker path override |
| `VPNHIDE_CONFIG_PATH` / `VPNHIDE_KMOD_CTL` / `VPNHIDE_PORTS` | Keep **explicit** enablement for writes |

---

## 10. Still required before production writes

1. APK (and full reset) refuse canonical writes when `config_owned` is present.
2. Common Magisk module boot lifecycle (step 5) starts core before legacy `service.sh` activators; or disable those scripts once markers are claimed.
3. Device smoke: adopt → claim → legacy activator no-op → core Persist/Native/Ports.
4. Do **not** default-enable `VPNHIDE_PORTS` / ctl writes until (1)–(3) hold.
5. Rollback/disable story for markers + journal retention (documented in architecture §11; packaging owns the scripts).

## Readiness for step 5

Inventory + fencing hooks are in place for activator/uninstall boundaries and config
load distinctions. Step 5 (common Magisk module + boot lifecycle) can wire core
start + `VPNHIDE_CORE_ADOPT` / marker dir into module scripts without flipping
production write defaults yet.
