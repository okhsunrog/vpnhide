# vpnhide — storage & activation architecture

How configuration is **stored**, who **reads** it, and how it is **activated** into
each backend. This is the layer *above* the wire protocol: [protocol.md](protocol.md)
defines the bytes exchanged with the kernel/native backends at runtime; this file
defines the single on-disk source of truth those bytes are derived from.

Proposed app-side state and synchronization changes are described in
[App state redesign](app-state-design.md). That document is a design proposal;
the storage and activation behavior below describes the current implementation.

> **Status.** This is the current storage/activation design: one canonical JSON
> desired-state file, Rust activators that derive runtime state for native and
> ports backends, and LSPosed reading the JSON directly from `system_server`.
> Per-backend text files exist only as derived runtime state, never as
> user-managed config.

---

## 1. Two layers, one boundary

Two formats, by function — not one format stretched over everything:

- **On disk = JSON.** A single canonical "desired state" file. Package-keyed (so it
  survives reinstalls), rich (roles, per-hook selection, debug, app settings),
  trivially import/export-able. Read by the app (Kotlin), the LSPosed hook
  (Kotlin), and the Rust activators.
- **Runtime IPC = the text protocol** ([protocol.md](protocol.md)): control v2
  for config, telemetry v1 for stats/status, both frozen. It is uid-keyed,
  hand-parsed, and kernel-safe. All native backends consume control v2; kmod and
  KPM emit telemetry v1. LSPosed also emits telemetry-shaped status/stats (§3)
  but reads desired state directly from JSON.
- **The bridge = the activator** (Rust). It projects the JSON onto the wire for
  the selected native backend.

Why split: the kernel consumer (KPM, freestanding C) cannot parse JSON (see
protocol.md §9), and the injected Zygisk `.so` should not carry a JSON parser into
every app process. But the *storage* layer has no such constraint, and JSON buys
one-file import/export and natural Kotlin/Rust structs. So JSON serves the high
level (storage + the Java hook, where `org.json` is free), the text protocol serves
the low level (kernel / injected native), and the activator translates between
them. Direct raw debugging over `adb` (protocol.md §2) is preserved:
the live kernel state is still `cat /proc/vpnhide_ctl` text, and JSON is itself
readable text.

The number-keying difference is deliberate and bridged by the activator:

| Layer | Keyed by | Why |
|---|---|---|
| Canonical JSON | **package name** | survives reinstall (UID rotates, package doesn't) |
| Runtime protocol | **UID** | the kernel has no PackageManager |

VPN Hide itself is the one package with a deliberately narrower projection:
the activator emits only its main-profile (user 0) UID. The APK blocks its UI in
secondary/work/clone profiles, so accidental extra copies neither write the
shared canonical file nor consume extra native target slots.

---

## 2. The canonical config (the only file you manage)

- **Path:** `/data/system/vpnhide_config.json`
- **Perms / label:** `0640 root:system`, SELinux `system_data_file`.
- **Writer:** the app, via `su` (atomic `tmp`+`rename`).
- **Readers:** the app (`su`), the LSPosed hook (directly, from `system_server`),
  the activator (root). **Not** readable by unprivileged apps or non-root `adb`
  (see §7).
- **Holds:** schema version, the global `debug` flag (effective logging intent),
  `debugSwitch` (user toggle intent), per-package roles + (optional) per-hook
  selection, and app settings. It does **not** hold stats, status, or the APatch
  superkey (those are runtime/secret — §5, §6).
- **Import / export = copy this one file.** It contains no device-specific secret,
  so it is safe to back up / move / share. (This is *why* the superkey is **not**
  stored here — §6.)
- **Pre-1.0 configs are imported into it, not read alongside it.** Installs older
  than 1.0.0 kept per-component `targets.txt`/observer files; the app folds them
  into this JSON and deletes them (silently at startup when nothing is configured
  yet, otherwise via a Dashboard banner offering merge / replace). Paths and
  exact behaviour: [state.md §1](state.md), code in `LegacyConfigImport.kt`.
  Nothing outside that importer reads them — no backend has a legacy read path.

### 2.1 Canonical JSON schema

Shape (illustrative):

```json
{
  "version": 1,
  "debug": false,
  "debugSwitch": false,
  "apps": {
    "com.example.bank":  { "java": true, "native": true,  "appHiding": false, "ports": false },
    "org.example.proxy": {
      "java": ["lsposed_network_capabilities"],
      "native": true,
      "appHiding": true,
      "ports": true,
      "portPolicy": {
        "mode": "preset",
        "preset": "common_proxy",
        "rules": [
          { "protocol": "both", "start": 1080 },
          { "protocol": "both", "start": 7890, "end": 7892 }
        ]
      }
    }
  },
  "settings": {
    "rememberSuperkey": false,
    "optionalFeatures": []
  }
}
```

`debugSwitch` is the user intent stored by the app toggle; `debug` is the
effective in-runtime value. On a capture path, `debug` may temporarily become
`true` while `debugSwitch` remains `false`. Startup reconciliation restores
`debug := debugSwitch` when they diverge.

Migration from pre-switch configs:

- If `debugSwitch` is missing, it is treated as `debug` while parsing so older
  canonical JSONs keep working.

Per-hook granularity ("раздельное управление хуками"): the coarse `"java": true`
or `"native": true` is the common case (enable every hook in that role for the
app). Java per-hook control uses an explicit hook list, e.g.
`"java": ["lsposed_network_capabilities"]`. Native per-hook control is
backend-family-specific because `.ko`/KPM kernel hooks and Zygisk libc hooks are
different hook ids:

```json
"native": {
  "enabled": true,
  "kernel": ["fib_route_seq_show", "sock_ioctl"],
  "zygisk": ["zygisk_ioctl", "zygisk_recvfrom_chk"]
}
```

The old `"native": ["fib_route_seq_show", "sock_ioctl"]` shape is still accepted
as a legacy **kernel** override. For any active native backend family whose list
is absent, the role means "all hooks for that family"; switching from Zygisk to
KPM preserves the Zygisk override but uses all kernel hooks until the user
configures kernel hooks explicitly, and vice versa. The native activator maps the
active family list to protocol `hookmask` bits. LSPosed reads the Java list
directly in `system_server`; app-hiding package visibility is still controlled
by `appHiding`, not by the Java VPN hook list. The JSON is the *desired*
selection; the native wire carries the resolved `hookmask`.

The `rememberSuperkey` boolean lives here (it is a preference, not a secret). The
superkey itself does **not** (§6).

### 2.2 Optional features

`settings.optionalFeatures` is a canonical **desired-state field**, not a
control-v2 record. It names expensive or experimental capabilities whose
backend-specific activation is defined here. Unknown names are preserved so
newer versions can extend the set without a schema migration.

The current projection is:

| Canonical feature | `.ko` loader ABI | KPM loader ABI | Zygisk projection |
|---|---|---|---|
| `filesystem_iface_paths` | `insmod vpnhide_kmod.ko filesystem_hiding=1` | load `vpnhide.kpm` with the exact argument `filesystem_hiding=1` | include hook bit 27 in each selected target's Zygisk mask |

When the feature is absent, the shipped `.ko` loader passes
`filesystem_hiding=0`; the KPM loader omits the argument. Either choice is made
before hook installation, so changing the canonical set takes effect only after
a reboot. Backend-specific parsing and failure behavior are documented in the
[`.ko` README](../kmod/README.md#optional-filesystem-hook-loader-contract) and
[KPM README](../kmod/kpm/README.md#optional-filesystem-hook-loader-contract).
For Zygisk, the activator instead projects the feature into each target's
module-dir control snapshot. A force-stop and restart of the target process is
enough to pick up the change; no reboot or loader argument is involved.

Filesystem hiding has two independent gates on kernel backends:

1. The boot feature installs the four global VFS interception points. With the
   feature off, the backend reports no `filesystem_iface_paths` capability and
   pays no VFS hot-path trampoline cost.
2. Control v2 hook bit 27 selects target UIDs allowed to use that installed
   capability. Installing the hooks alone does not hide anything for a UID whose
   target mask omits the bit.

Zygisk uses the same canonical feature and hook ID, but its first gate is an
atomic group of process-local libc hooks installed during app specialization.
The heartbeat records both the requested and successfully installed masks; if
any filesystem symbol cannot be hooked, bit 27 is omitted from the installed
mask while the ordinary Zygisk network hooks remain active. This is deliberately
best-effort coverage rather than the kernel backends' resolved-dentry contract.

The symbolic feature name and loader arguments belong to storage/activation;
hook ID 27, its mask semantics, and its installed/status bit belong to the
[wire protocol](protocol.md#5-hook-registry-global-id-space).

Ports are intentionally controlled in the canonical JSON, not in the native text
protocol. `"ports": true` with no `portPolicy` is the legacy/default behavior:
the ports activator blocks all TCP/UDP connections from that app UID to
`127.0.0.1` and `::1`. When `portPolicy` is present, `rules` are the materialized
source of truth for import/export and activation; `mode`/`preset` are UI metadata.
Each rule uses `protocol: "both" | "tcp" | "udp"`, `start`, and optional `end`
(inclusive, `1..65535`). Presets may change in later app versions, but an exported
config remains stable because it carries the resolved `rules`.

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
  `packages.list` is a plain file read, hook-free. The same self-read + `packages.list`
  resolution also drives the **Apps** role (package visibility) — it just keys on the
  observer UIDs and hidden packages instead of the target set, so both LSPosed jobs
  share one hook-free resolver.
- **Multi-profile for free:** match `callingUid % 100000 ∈ targetAppIds`. Any
  profile's instance of a target app matches, with no per-profile enumeration.
- **State out:** the hook writes `/data/system/vpnhide_lsposed_state`, a single
  protocol-shaped readback file containing `vpnhide 1 status` plus LSPosed
  metadata (`version`/`boot_id`/`broken_fields`) and `vpnhide 1 stats` counters.
  Config-in and state-out are still different files / opposite directions:
  `system_server` can't expose a `/proc`-style node, so it can't multiplex write
  config + read state on one channel the way the kmod node does.

Older releases used a derived `vpnhide_uids.txt` file for this path. Current code
does not: the hook self-reads the canonical JSON and resolves UIDs in-process.

---

## 4. The activator and its backends (native + ports)

The native backends are designed to be **mutually exclusive**: the supported
installation has exactly one of kmod, KPM, or Zygisk. The app warns or errors and
asks the user to remove extras. A Save invokes only the highest-priority enabled
native activator; boot scripts remain module-local, which is why extra installed
modules must still be removed rather than treated as idle replicas.

### 4.1 The activator — a Rust workspace, thin bins

The projection is shared for all three native backends (take the `native`-role
packages → resolve to UIDs → emit a `vpnhide 2 config` snapshot), but the
resolved hookmask uses the active backend family: `.ko`/KPM consume `kernel`
overrides, while Zygisk consumes `zygisk` overrides. Only the **delivery sink**
differs after that. Ports use the same canonical parser and package→UID
resolver, but project `ports: true` roles into iptables rules instead of the text
wire. So: one shared core, thin per-target front-ends.

```
crates/
  apatch-abi/                # read-only/load supercall command compatibility,
                             #   shared by the KPM activator and APK probe.
  protocol/                 # lib, NO serde — the wire (parse/format), shared by
                            #   the Zygisk .so and the activator.
  activator/
    src/lib.rs              # shared core: JSON schema (serde), pkg→uid resolution
                            #   (`pm`), project_native(json) -> String
    src/lifecycle.rs        # typed boot load/status/service/uninstall ownership
    src/bin/kmod.rs         #   project_native(json) → write("/proc/vpnhide_ctl")
    src/bin/kpm.rs          #   project_native(json) → APatch/FolkPatch supercall / KPatch-Next `kpatch`
    src/bin/zygisk.rs       #   project_native(json) → write_atomic(module_dir file)
    src/bin/ports.rs        #   activate_ports() → iptables-restore/ip6tables-restore
zygisk/                     # cdylib — the injected .so. deps: protocol (+ shadowhook).
                            #   Lean: no serde, no JSON in every app process.
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
- **When it runs:** root-manager lifecycle files are fixed-name shell adapters.
  Blocking `post-fs-data.sh` and uninstall hooks `exec` `boot-load` or
  `uninstall`; every `service.sh` backgrounds `boot-service` and returns so one
  module cannot starve the manager's sequential late-start script runner.
  Early `boot-load` does bounded backend loading only; it never waits for
  PackageManager or user unlock. Late-start `boot-service` may wait indefinitely
  for PackageManager readiness, while Save (`su <path>/activator`) keeps a bounded
  wait so the UI cannot hang forever. It lists Android users first and
  resolves every user separately instead of relying on the OEM-dependent
  `--user all` aggregation. If any user scan fails, activation stops before
  replacing runtime state with a partial target set — this is the native
  activator's own (Rust/C) runtime save-safety and is unrelated to the app
  UI below. Then it reads the canonical and writes its channel.
  The app-side target picker enumerates the same way but is fail-soft: a
  failed profile scan (other than user 0, which also gets an in-process
  `getInstalledApplications(0)` backstop) shows a banner instead of blocking
  the whole app list, and Save preserves settings for packages it couldn't
  see. Root can read every profile regardless of lock state, so the picker
  only hard-fails when nothing could be enumerated from any source at all.
- **Bundle integrity:** the app's batched root snapshot checks each installed
  module's `activator` directly and distinguishes an absent file from a
  non-executable one. Enabled modules with either failure are marked broken on
  the Dashboard. Save selects the first installed, enabled native backend in
  the normal `kmod > KPM > Zygisk` order and fails if that backend's activator
  is unusable; it never silently falls through to another backend.
  KPM remediation means installing the complete `vpnhide-kpm.zip` through the
  root manager's Modules screen, not extracting or loading the inner
  `vpnhide.kpm` payload by itself.
- **KPM load preflight:** the KPM activator parses only the leading numeric
  `major.minor` from `uname -r`; patchlevels and Android/vendor suffixes do not
  affect the offset-table family. It refuses an unknown or unsupported family
  before invoking KernelPatch, and the KPM repeats the authoritative check in
  kernel context. KPatch-Next uses `activator boot-load` during post-fs-data
  so this validation does not wait for PackageManager.

### 4.2 The native channels

| Backend | Channel | Kind | Note |
|---|---|---|---|
| kmod | `/proc/vpnhide_ctl` | proc node | write = config (kernel parses to memory); read = status+stats |
| KPM | KPM ctl0 supercall | supercall | APatch/FolkPatch direct supercall or KPatch-Next runtime `kpatch kpm ctl0`; no file |
| Zygisk | file in its module dir | file | the `.so` reads it via the `get_module_dir()` fd |

Why Zygisk needs a file in its module dir even though there is one canonical: the
`.so`'s **only** privileged read handle per fork is the module-dir fd Zygisk hands
it (§7). This is a *mechanism* constraint, not a format one — the activator simply
derives the protocol config there. (Note: the protocol text — not JSON — keeps the
injected `.so` free of a JSON parser and carries the per-hook `hookmask` + `debug`
that a flat package list could not express.)

### 4.3 Native backend selection and safety

The Java and native layers are orthogonal: LSPosed may run alongside one native
backend, while the native backend is exactly one of `.ko`, KPM, or Zygisk. On
Save, the app runs at most one installed and enabled native activator in fixed
priority order `kmod > KPM > Zygisk`. It does not fan one snapshot out to every
installed backend and does not use empty snapshots as a selection mechanism.
Multiple installed native modules are a configuration issue that the dashboard
asks the user to resolve; `.ko` plus KPM is elevated to an error because that
overlap is unsafe.

The `.ko` and KPM restriction is also a kernel safety boundary. They wrap the
same kernel functions with different mechanisms (kretprobes versus KernelPatch
inline hooks); co-residence has hard-frozen a device. Every shipped KPM load and
configuration path therefore refuses when an enabled `.ko` module directory or
live `/proc/vpnhide_ctl` is present and reports `conflicting_backend`. The check
runs in both activator phases to close the early-boot race.
Manual loading outside those paths bypasses the guard and is unsupported.

There is no resident root coordinator. Save-time activation and each module's
boot scripts perform the rare projection/apply operation, while every backend
owns its own runtime channel. This avoids a persistent enumerable process and
keeps channel permissions and lifetimes backend-specific.

---

## 5. Stats & status — and what "hooks" means

### 5.1 `config` hookmask vs `status` hooks — two different axes

A common confusion (they are *not* the same):

- **`config`: `targets <hookmask> <uid>...`** — grouped per-UID intent: which hooks are
  enabled for that app. This is the flexible per-target control.
- **`status`: `hooks <mask>`** — per-backend, what is *actually installed*: did all
  the backend's hooks register this boot? It is capability/health, not per-target.
  Lets the app show "requested vs active" and detect a partial install
  (`error = partial_hooks`). So `hooks 0x20003ff` in a status read means "all 11
  shared kernel hooks are installed in this backend"; when filesystem hiding is
  active, bit 27 is also present. This is **not** a target's hook selection.

### 5.2 Config and stats don't collide

They are **opposite directions of one channel**, distinguished by the `kind`
header — not interleaved in one stored blob. Write feeds config (into memory); read
emits status+stats (from separate counters). Example on the kmod node:

```sh
# write config (kind=config) — kernel parses into targets[]+debug, nothing echoed
# printf 'vpnhide 2 config\ndebug 0\ntargets 20003ff 27fa\nend 1\n' > /proc/vpnhide_ctl

# read status+stats (kind=status, kind=stats) — never returns the config you wrote
# cat /proc/vpnhide_ctl
# vpnhide v1 — a WRITE replaces ENTIRE state; this read is status+stats
vpnhide 1 status
backend 0x0
kver 0x6019d
hooks 0x20003ff
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
  no per-hit push) and needs a resident root collector, contrary to §4.3. Shared
  memory is the pull-compatible direction, but there is no clean way
  to share *writable* memory across all injected app-domain processes without a
  daemon (the module-dir file isn't writable by `untrusted_app` under SELinux; a
  zygote-inherited memfd must be created in the zygote, where our code does not run).
  So Zygisk consumes per-target hookmasks but does not emit per-hook stats yet;
  stats remain a future problem, not a blocker for this design.
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
Magisk/KSU is **keyless**; this whole section is **APatch/FolkPatch-only**.)

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
  (protection active after a reboot, no app open needed). Some FolkPatch/APatch
  builds expose a trusted `su` token to already-authorized root callers; on those
  builds the activator can use that token and no saved SuperKey is needed.
  Without either token, boot records `awaiting_superkey`.
- **Plain file, not Keystore.** The boot activator is a Rust binary, not an Android
  app, so it cannot call Android Keystore (a framework/binder API). Keystore-wrapping
  could protect the *app-time* use only, not the boot path. So the boot-usable copy
  is a plain file; FBE DE-encryption protects it at rest on a powered-off device.
- **Threat-model fit:** readable by **root only** (trusted in this project's model),
  never by unprivileged apps or non-root `adb` (§7). The APatch/FolkPatch activator
  calls the supercall directly, so the key is not passed through an external
  `kpatch` argv.

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

## 8. File inventory

What you actually manage shrinks to **one** file; everything else is generated or
optional.

| File | Role | Lifetime |
|---|---|---|
| `/data/system/vpnhide_config.json` | **the** canonical config (managed, exported) | persistent |
| `/proc/vpnhide_ctl` | kmod runtime channel (node, not a file) | per-boot, in-kernel |
| KPM ctl0 supercall | KPM runtime channel (supercall, no file) | per-boot, in-kernel |
| `/data/adb/modules/vpnhide_zygisk/targets.txt` | Zygisk runtime channel (derived copy) | regenerated |
| `/data/system/vpnhide_lsposed_state` | LSPosed status + stats (hook → dashboard) | per-boot |
| `/data/adb/vpnhide/superkey` | APatch superkey (optional, flag-gated) | persistent, root-only |

## 9. Contract ownership

Each concern has one authoritative home; cross-links provide context without
redefining the other contracts:

| Concern | Authoritative document |
|---|---|
| Canonical JSON schema and desired state | this document (§2) |
| JSON projection, native selection, and activation policy | this document (§4) |
| Persistent paths, diagnostic files, lifetimes, and boot sequence | [state.md](state.md) |
| Control-v2 / telemetry-v1 bytes, registries, and runtime framing | [protocol.md](protocol.md) |
| `.ko` module parameters and hook-install behavior | [kmod README](../kmod/README.md) |
| KPM load arguments and KernelPatch runtime behavior | [KPM README](../kmod/kpm/README.md) |

LSPosed reads canonical JSON directly (§3), while the Rust activator serialises
control v2 for the selected native backend (§4). The app persists desired state
and invokes that activator; it does not hand-build per-channel wire payloads.
