# vpnhide — storage & activation architecture

How configuration is **stored**, who **reads** it, and how it is **activated** into
each backend. This is the layer *above* the wire protocol: [protocol.md](protocol.md)
defines the bytes exchanged with the kernel/native backends at runtime; this file
defines the single on-disk source of truth those bytes are derived from.

> **Status.** This is the **target** design. The current code (the control-protocol
> migration, PR #164) is an interim step: it puts the text protocol on *every*
> backend with per-backend text files. This document is where that converges —
> one JSON canonical + activators that derive runtime state for the native and
> ports backends, with LSPosed reading the JSON directly. Sections below note
> where the current code differs.

---

## 1. Two layers, one boundary

Two formats, by role — not one format stretched over everything:

- **On disk = JSON.** A single canonical "desired state" file. Package-keyed (so it
  survives reinstalls), rich (roles, per-hook selection, debug, app settings),
  trivially import/export-able. Read by the app (Kotlin), the LSPosed hook
  (Kotlin), and the Rust activators.
- **Runtime IPC = the text protocol** ([protocol.md](protocol.md), v1 frozen).
  uid-keyed, hand-parsed, kernel-safe. The runtime wire for the **native**
  backends only (kmod / KPM / Zygisk): config in, stats/status out.
- **The bridge = the activator** (Rust). It projects the JSON onto the wire for
  whichever native backend is installed.

Why split: the kernel consumer (KPM, freestanding C) cannot parse JSON (see
protocol.md §9), and the injected Zygisk `.so` should not carry a JSON parser into
every app process. But the *storage* layer has no such constraint, and JSON buys
one-file import/export and natural Kotlin/Rust structs. So JSON serves the high
level (storage + the Java hook, where `org.json` is free), the text protocol serves
the low level (kernel / injected native), and the activator translates between
them. The "agent reads/writes raw over adb" property (protocol.md §2) is preserved:
the live kernel state is still `cat /proc/vpnhide_ctl` text, and JSON is itself
readable text.

The number-keying difference is deliberate and bridged by the activator:

| Layer | Keyed by | Why |
|---|---|---|
| Canonical JSON | **package name** | survives reinstall (UID rotates, package doesn't) |
| Runtime protocol | **UID** | the kernel has no PackageManager |

---

## 2. The canonical config (the only file you manage)

- **Path:** `/data/system/vpnhide_config.json`
- **Perms / label:** `0640 root:system`, SELinux `system_data_file`.
- **Writer:** the app, via `su` (atomic `tmp`+`rename`).
- **Readers:** the app (`su`), the LSPosed hook (directly, from `system_server`),
  the activator (root). **Not** readable by unprivileged apps or non-root `adb`
  (see §7).
- **Holds:** schema version, the global `debug` flag, per-package roles + (optional)
  per-hook selection, and app settings. It does **not** hold stats, status, or the
  APatch superkey (those are runtime/secret — §5, §6).
- **Import / export = copy this one file.** It contains no device-specific secret,
  so it is safe to back up / move / share. (This is *why* the superkey is **not**
  stored here — §6.)

Shape (illustrative):

```json
{
  "version": 1,
  "debug": false,
  "apps": {
    "com.example.bank":  { "java": true, "native": true,  "appHiding": false, "ports": false },
    "org.example.proxy": { "java": true, "native": true,  "appHiding": true,  "ports": true }
  },
  "settings": {
    "rememberSuperkey": false
  }
}
```

Per-hook granularity ("раздельное управление хуками"): the coarse `"native": true`
is the common case (enable every native hook for the app). When the UI exposes
per-hook control, `native` grows into an explicit hook list, e.g.
`"native": ["fib_route_seq_show", "sock_ioctl"]`; the activator maps that to the
protocol `hookmask` bits. An absent/`true` value means "all native hooks". The JSON
is the *desired* selection; the wire carries the resolved `hookmask`.

The `rememberSuperkey` boolean lives here (it is a preference, not a secret). The
superkey itself does **not** (§6).

---

## 3. Java layer (LSPosed) — independent, self-reading

The LSPosed backend is an **APK + a `system_server` hook**, not a Magisk module. Two
consequences drive the design:

1. **It has no `service.sh`** — nothing external should "populate" a file for it.
2. **It must work standalone.** The native modules are optional and
   mutually-exclusive plug-ins; the Java layer must be fully functional with *zero*
   native modules installed. So it cannot depend on a native module's boot script to
   configure it.

The resolution: the **hook itself is the Java layer's boot mechanism** — LSPosed
loads it into `system_server` on every boot. So it reads the canonical config
**directly** and configures itself. No activator, no derived file.

How it reads, carefully (this runs in the bootloop-critical `system_server`):

- **Config in:** read `/data/system/vpnhide_config.json` directly (the hook is in
  `system_server`, which *can* read `system_data_file`). `org.json` is built into
  Android — no dependency. Re-read on `FileObserver` change. `debug` comes from the
  same JSON (no separate debug file).
- **UID resolution without re-entry:** the hook matches the *caller* by UID
  (`Binder.getCallingUid()`), so it needs the target packages' UIDs. It resolves
  them via **`/data/system/packages.list`** (the stable AOSP `pkg uid …` map) —
  **not** via `PackageManager`, because the hook *is* hooking PackageManager and a
  PM call could re-enter its own hooks (recursion → deadlock → bootloop). Reading
  `packages.list` is a plain file read, hook-free.
- **Multi-profile for free:** match `callingUid % 100000 ∈ targetAppIds`. Any
  profile's instance of a target app matches, with no per-profile enumeration.
- **State out:** the hook writes `/data/system/vpnhide_lsposed_state`, a single
  protocol-shaped readback file containing `vpnhide 1 status` plus LSPosed
  metadata (`version`/`boot_id`/`broken_fields`) and `vpnhide 1 stats` counters.
  Config-in and state-out are still different files / opposite directions:
  `system_server` can't expose a `/proc`-style node, so it can't multiplex write
  config + read state on one channel the way the kmod node does.

> Current code (#164) differs: LSPosed reads a *derived protocol config*
> (`vpnhide_uids.txt`) that the native boot scripts populate. That is exactly the
> native-dependency this target design removes.

---

## 4. Native layer (kmod / KPM / Zygisk) + the activator

The native backends are **mutually exclusive** — exactly one is installed (the app
warns/errors and asks the user to remove the extra otherwise). So the activator
never writes "all three": it writes the **one** channel of the one installed native
backend.

### 4.1 The activator — a Rust workspace, thin bins

The projection is *identical* for all three native backends (take the `native`-role
packages → resolve to UIDs → emit a `vpnhide 1 config` snapshot); only the
**delivery sink** differs. Ports use the same canonical parser and package→UID
resolver, but project `ports: true` roles into iptables rules instead of the text
wire. So: one shared core, thin per-target front-ends.

```
crates/
  protocol/                 # lib, NO serde — the wire (parse/format). Shared by
                            #   the Zygisk .so AND the activator. Today's
                            #   zygisk/src/protocol.rs, promoted to a shared crate.
  zygisk/                   # cdylib — the injected .so. deps: protocol (+ shadowhook).
                            #   Lean: no serde, no JSON in every app process.
  activator/
    src/lib.rs              # shared core: JSON schema (serde), pkg→uid resolution
                            #   (`pm`), project_native(json) -> String
    src/bin/kmod.rs         #   project_native(json) → write("/proc/vpnhide_ctl")
    src/bin/kpm.rs          #   project_native(json) → APatch supercall / KPatch-Next `kpatch`
    src/bin/zygisk.rs       #   project_native(json) → write_atomic(module_dir file)
    src/bin/ports.rs        #   project_ports(json) → iptables-restore/ip6tables-restore
```

- **Why a workspace, not feature-flags or one crate with `src/bin/`+cdylib:**
  `cdylib` and `bin` are different crate-types; feature-flags don't select artifact
  type. A single crate that is *both* a cdylib and a bin risks pulling `serde` into
  the injected `.so`. The workspace keeps the `.so`'s dependency tree lean (protocol
  only) while the activator (a normal executable) freely uses `serde`. The
  **activators are executables**, so they fit `src/bin/` perfectly over a shared
  `src/lib.rs`.
- **Each native module ships only its own activator bin** (kmod module → `kmod`,
  KPM module → `kpm`, Zygisk module → `zygisk`). Each does exactly its one channel,
  no detection, no runtime branching.
- **The ports module ships `ports`**, which reads the same canonical JSON but applies
  iptables state instead of writing the native text protocol.
- **When it runs:** at boot from that module's `service.sh` as
  `activator --boot-wait`, and on Save from the app (`su <path>/activator`).
  Boot mode waits indefinitely for PackageManager readiness; Save mode keeps a
  bounded wait so the UI cannot hang forever. Then it reads the canonical,
  resolves, and writes its channel.

### 4.2 The native channels

| Backend | Channel | Kind | Note |
|---|---|---|---|
| kmod | `/proc/vpnhide_ctl` | proc node | write = config (kernel parses to memory); read = status+stats |
| KPM | KPM ctl0 supercall | supercall | APatch direct supercall or KPatch-Next runtime `kpatch kpm ctl0`; no file |
| Zygisk | file in its module dir | file | the `.so` reads it via the `get_module_dir()` fd |

Why Zygisk needs a file in its module dir even though there is one canonical: the
`.so`'s **only** privileged read handle per fork is the module-dir fd Zygisk hands
it (§7). This is a *mechanism* constraint, not a format one — the activator simply
derives the protocol config there. (Note: the protocol text — not JSON — keeps the
injected `.so` free of a JSON parser, and carries the per-hook `hookmask` + `debug`
+ the native stats the way a flat package list never could.)

---

## 5. Stats & status — and what "hooks" means

### 5.1 `config` hookmask vs `status` hooks — two different axes

A common confusion (they are *not* the same):

- **`config`: `target <uid> <hookmask>`** — per-UID, what you *want*: which hooks are
  enabled for that app. This is the flexible per-target control.
- **`status`: `hooks <mask>`** — per-backend, what is *actually installed*: did all
  the backend's hooks register this boot? It is capability/health, not per-target.
  Lets the app show "requested vs active" and detect a partial install
  (`error = partial_hooks`). So `hooks 0x3ff` in a status read means "all 10 kernel
  hooks are installed in this backend", **not** "this target's hooks".

### 5.2 Config and stats don't collide

They are **opposite directions of one channel**, distinguished by the `kind`
header — not interleaved in one stored blob. Write feeds config (into memory); read
emits status+stats (from separate counters). Example on the kmod node:

```sh
# write config (kind=config) — kernel parses into targets[]+debug, nothing echoed
# printf 'vpnhide 1 config\ndebug 0\ntarget 0x27fa 0x3ff\n' > /proc/vpnhide_ctl

# read status+stats (kind=status, kind=stats) — never returns the config you wrote
# cat /proc/vpnhide_ctl
# vpnhide v1 — a WRITE replaces ENTIRE state; this read is status+stats
vpnhide 1 status
backend 0x0
kver 0x6019d
hooks 0x3ff
error 0x0
vpnhide 1 stats
0x27fa 0x0:0x5 0x3:0xc 0x9:0x1
```

`ctl_write` touches `targets[]`; `ctl_show` serialises counters — different structs,
different syscalls, own locks. For file channels (Zygisk) the config-in file is
single-writer / atomic-replace, and stats are never written back into it.

### 5.3 Stats scope

- **kmod / KPM: yes** — counters live in one place (kernel memory), so a
  pull-read is natural. They emit cumulative-since-load non-zero
  `(uid, hook_id)` cells for hooks that actually hid or rewrote a result.
- **Zygisk: deferred.** Its counters would live **per app process** with no shared
  aggregation point. A unix socket is out — it violates protocol.md §2 (pull-only,
  no per-hit push) and needs a collector process (a rejected resident root daemon,
  §1.1). Shared memory is the pull-compatible direction, but there is no clean way
  to share *writable* memory across all injected app-domain processes without a
  daemon (the module-dir file isn't writable by `untrusted_app` under SELinux; a
  zygote-inherited memfd must be created in the zygote, where our code does not run).
  So Zygisk emits **status only**; per-hook stats are a future problem, not a
  blocker for this design.
- **LSPosed: yes** — counters live in `system_server`, so the hook aggregates
  cumulative-since-boot non-zero `(uid, hook_id)` cells and writes them into
  `/data/system/vpnhide_lsposed_state` next to its `status` block.

---

## 6. APatch SuperKey

KernelPatch's control entry is a **syscall** (the supercall), and syscalls are not
UID-gated — *any* process could invoke it. The **superkey** is the capability token
that authenticates "may control the kernel patch". So it defends against the same
**unprivileged** adversary this project already cares about: without it, a non-root
app could `ctl0` our config or grant itself root. (KPatch-Next `d05` under
Magisk/KSU is **keyless**; this whole section is **APatch-only**.)

- **Default:** session-only, re-prompt each session (APatch's own model).
- **Optional flag `rememberSuperkey`** (in the canonical `settings`): persist it so
  it loads automatically — convenient for users who prefer it, and for development
  (no manual entry each time).
- **When the flag is on**, the key is stored at:
  - **Path:** `/data/adb/vpnhide/superkey`, `0600 root`.
  - **Why there, not elsewhere:**
    - **Not in the canonical JSON** — that is the export file; the key would leak on
      export, and it is device-specific (useless on another device) anyway.
    - **Not in CE app-private** (`/data/data/<pkg>`, credential-encrypted) — that is
      locked until the user unlocks the device, so a **boot** script (which runs
      *before* first unlock) can't read it.
    - **`/data/adb` is DE-accessible** (device-encrypted, decrypted at boot — Magisk
      reads modules from there pre-unlock), and root-only. So the boot activator
      (root) **can** read it before unlock.
- **What it unlocks: APatch boot activation.** With the key on disk, the `kpm`
  activator reads it at boot and uses APatch's KernelPatch supercall ABI directly
  to load/configure the KPM — so APatch reaches **parity with keyless KPatch-Next**
  (protection active after a reboot, no app open needed). Without the flag, APatch
  cannot be configured at boot because the KPM supercalls require the real key.
- **Plain file, not Keystore.** The boot activator is a Rust binary, not an Android
  app, so it cannot call Android Keystore (a framework/binder API). Keystore-wrapping
  could protect the *app-time* use only, not the boot path. So the boot-usable copy
  is a plain file; FBE DE-encryption protects it at rest on a powered-off device.
- **Threat-model fit:** readable by **root only** (trusted in this project's model),
  never by unprivileged apps or non-root `adb` (§7). The APatch activator calls the
  supercall directly, so the key is not passed through an external `kpatch` argv.

---

## 7. SELinux & visibility — who can read what

Threat model: an **unprivileged** app; we do **not** hide from root. The whole layout
falls out of one fact — three reader contexts with different reach:

| Reader | Context | Can reach |
|---|---|---|
| app | `su` (root) | anything |
| activator / boot scripts | root | anything |
| LSPosed hook | `system_server` domain, **no root** | `system_data_file` only — **not** `/data/adb` |
| Zygisk `.so` | per-fork, module-dir **fd** only | only what that fd points at |

Consequences:

- **Canonical → `/data/system`** (`system_data_file`, `0640 root:system`): app(su) +
  activator(root) + LSPosed(`system_server`) all read it; unprivileged apps and
  non-root `adb` get EACCES. (The one place `system_server` can read.)
- **Superkey → `/data/adb`** (`0600 root`, DE): root-only and boot-readable.
- **Zygisk → a copy in its module dir**: because its only privileged handle is the
  module-dir fd. A direct `open()` of the canonical from the zygote domain would need
  a fragile `sepolicy.rule`; a companion would be a rejected daemon. So: a cheap
  derived copy, written by the activator.

Visibility of the two sensitive files, concretely:

| Reader | `/data/system/vpnhide_config.json` (0640 root:system) | `/data/adb/vpnhide/superkey` (0600 root) |
|---|---|---|
| unprivileged app | no | no |
| root app | yes | yes |
| `adb shell` (no root) | **no** (DAC + SELinux) | **no** |
| `adb` with root (userdebug) | yes | yes |
| `system_server` | yes (group `system`) | no |

So `adb` without root reads **neither**; both are root-only beyond that — which is
exactly the threat model.

---

## 8. File inventory (target)

What you actually manage shrinks to **one** file; everything else is generated or
optional.

| File | Role | Lifetime |
|---|---|---|
| `/data/system/vpnhide_config.json` | **the** canonical config (managed, exported) | persistent |
| `/proc/vpnhide_ctl` | kmod runtime channel (node, not a file) | per-boot, in-kernel |
| KPM ctl0 supercall | KPM runtime channel (supercall, no file) | per-boot, in-kernel |
| `/data/adb/modules/vpnhide_zygisk/<cfg>` | Zygisk runtime channel (derived copy) | regenerated |
| `/data/system/vpnhide_lsposed_state` | LSPosed status + stats (hook → dashboard) | per-boot |
| `/data/adb/vpnhide/superkey` | APatch superkey (optional, flag-gated) | persistent, root-only |

Removed vs. the pre-redesign layout:

- the four per-backend `targets.txt` lists → folded into the **one** canonical;
- `vpnhide_debug_logging` → **gone** (debug is a field in the canonical);
- `vpnhide_uids.txt` / `vpnhide_hidden_pkgs.txt` / `vpnhide_observer_uids.txt` →
  **gone** (LSPosed reads the canonical + `packages.list` and derives them in-process).

---

## 9. Migration

On first run after the upgrade, if `/data/system/vpnhide_config.json` is absent, the
app builds it **once** from whatever old per-backend text files exist (package
lists, observers, hidden, debug flag), writes the canonical, then runs the
activator. After that the old files are unused; app startup removes the retired
legacy inputs best-effort once canonical JSON exists. The canonical is the single
source from then on. Keep the legacy read paths for a few public releases as a
migration shim, then remove them.

---

## 10. Relationship to [protocol.md](protocol.md)

protocol.md remains the frozen **v1 wire** specification, and is still the runtime
IPC for the native backends. This document supersedes its statements about the
*storage/activation layer*, specifically:

- **LSPosed does not consume the wire** — it reads the canonical JSON directly
  (§3). protocol.md's "LSPosed parses its config profile from its file" / the
  LSPosed rows in its channel + profile tables are superseded here.
- **APatch boot is configurable** when the superkey is persisted (§6); without
  it, boot records `awaiting_superkey` and activation resumes after the app
  supplies the key.
- **The app does not hand-build per-channel configs** — the activator does
  (§4). The serialiser is the Rust `protocol` crate, shared with the Zygisk `.so`.

The wire format itself (protocol.md §4–§5) is unchanged.
