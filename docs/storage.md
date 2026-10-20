# vpnhide — storage & activation architecture

How configuration is **stored**, who **reads** it, and how it is **activated** into
each backend. This is the layer *above* the wire protocol: [protocol.md](protocol.md)
defines the bytes exchanged with the kernel/native backends at runtime; this file
defines the single on-disk source of truth those bytes are derived from.

App-side state ownership (config operations, observations, diagnostic runs) is
documented in [config coordinator](config-coordinator.md),
[observation coordinator](observation-coordinator.md) and the
[transition contract](notes/app-state-transitions.md); the original proposal is
kept in [notes/app-state-design.md](notes/app-state-design.md). The storage and
activation behavior below describes the current on-disk implementation.

> **Status (2.0).** One canonical JSON desired-state file, written only by the
> VPN Hide Core root daemon; core derives runtime state for the one selected
> native backend and for Ports; LSPosed reads the JSON directly from
> `system_server`. Per-backend text files exist only as derived runtime state,
> never as user-managed config. The app has no `su` and runs no shell.

---

## 1. Two layers, one boundary

Two formats, by function — not one format stretched over everything:

- **On disk = JSON.** A single canonical "desired state" file. Package-keyed (so it
  survives reinstalls), rich (roles, per-hook selection, debug, app settings),
  trivially import/export-able. Read by core (Rust, which also writes it), the
  LSPosed hook (Kotlin), and the app through core.
- **Runtime IPC = the text protocol** ([protocol.md](protocol.md)): control v2
  for config, telemetry v1 for stats/status, both frozen. It is uid-keyed,
  hand-parsed, and kernel-safe. All native backends consume control v2; kmod and
  KPM emit telemetry v1. LSPosed also emits telemetry-shaped status/stats (§3)
  but reads desired state directly from JSON.
- **The bridge = core** (Rust). It projects the JSON onto the wire for the
  selected native backend.

Why split: the kernel consumer (KPM, freestanding C) cannot parse JSON (see
protocol.md §9), and the injected Zygisk `.so` should not carry a JSON parser into
every app process. But the *storage* layer has no such constraint, and JSON buys
one-file import/export and natural Kotlin/Rust structs. So JSON serves the high
level (storage + the Java hook, where `org.json` is free), the text protocol serves
the low level (kernel / injected native), and core translates between
them. Direct raw debugging over `adb` (protocol.md §2) is preserved:
the live kernel state is still `cat /proc/vpnhide_ctl` text, and JSON is itself
readable text.

The number-keying difference is deliberate and bridged by core:

| Layer | Keyed by | Why |
|---|---|---|
| Canonical JSON | **package name** | survives reinstall (UID rotates, package doesn't) |
| Runtime protocol | **UID** | the kernel has no PackageManager |

VPN Hide itself is the one package with a deliberately narrower projection:
core emits only its main-profile (user 0) UID. The APK blocks its UI in
secondary/work/clone profiles, so accidental extra copies neither write the
shared canonical file nor consume extra native target slots.

---

## 2. The canonical config (the only file you manage)

- **Path:** `/data/system/vpnhide_config.json`
- **Perms / label:** `0640 root:system`, SELinux `system_data_file`.
- **Writer:** core only (atomic `tmp`+`rename`), for each edit the app submits
  over the core socket (`config_replace`).
- **Readers:** core (root), the LSPosed hook (directly, from `system_server`),
  and the app through core's published config state. **Not** readable by
  unprivileged apps or non-root `adb` (see §7).
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
    "optionalFeatures": [],
    "autoHideVpnServices": true,
    "autoHideVpnName": true,
    "autoHideExcludedPackages": [],
    "autoHiddenPackages": ["com.example.vpnclient"]
  }
}
```

The `autoHide*` settings drive the automatic hiding of VPN apps from observers:
`autoHideVpnServices` and `autoHideVpnName` are the detection signals the user
can switch off, `autoHideExcludedPackages` are packages the user removed from
that automatic set, and `autoHiddenPackages` is the set the app last
materialised from those rules (re-derived on Refresh and at startup, written
only when it changed). A package's app-owned `hidden` role is the union of its
manual and automatic hiding.

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
configures kernel hooks explicitly, and vice versa. Core maps the
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
For Zygisk, core instead projects the feature into each target's
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
core's Ports apply blocks all TCP/UDP connections from that app UID to
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
   native modules installed. So it cannot depend on core's delivery to
   configure it.

The resolution: the **hook itself is the Java layer's boot mechanism** — LSPosed
loads it into `system_server` on every boot. So it reads the canonical config
**directly** and configures itself. No core delivery, no derived file.

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

## 4. Core and the backends (native + ports)

In 2.0 a single root daemon, **VPN Hide Core** (`vpnhide-core.zip`, module id
`vpnhide_core`, crate `crates/core-daemon`), owns every privileged step: it writes
the canonical config, selects and loads the native backend, delivers the wire to
it and applies the Ports iptables rules. The backend modules carry only their
payload (`.ko`, `.kpm`, Zygisk `.so`) and no scripts; the 1.x per-module
activators, the Ports module and the built-in companion module are gone. The app
reaches core over its socket and never runs `su`. The design record is
[notes/core-2.0-architecture.md](notes/core-2.0-architecture.md).

The native backends are **strictly mutually exclusive**, enforced once per boot
(`crates/core-daemon/src/native_boot.rs`). The candidates are every installed
kmod, KPM or Zygisk module (`module.prop` present, no `disable`, no `remove`), the
built-in driver when `/proc/vpnhide_ctl` reports backend `0x4`, a live kmod, and a
live KPM core did not load (APatch autoload, a hand-loaded `.kpm`). An installed
module and its own live payload count as one candidate.

- **More than one candidate:** core loads nothing, delivers config to nothing,
  retires Zygisk's targets and publishes `Conflict`. The Dashboard asks the user
  to keep one backend and reboot.
- **Exactly one:** core loads it (or adopts it when it is already live).
- **None:** `NoBackend`; only LSPosed and Ports protect.

The decision is persisted with the kernel `boot_id` (`native_boot.json`) and is
fixed for the whole boot. A later Save or `RepairActivation` delivers config only
to the selected backend; a core restart in the same boot re-adopts it. The one
exception is a KPM waiting for authentication, which core retries once the
SuperKey is saved.

### 4.1 Workspace layout

The projection is shared by all native backends (take the `native`-role
packages → resolve to UIDs → emit a `vpnhide 2 config` snapshot), but the
resolved hookmask uses the active backend family: `.ko`/built-in/KPM consume
`kernel` overrides, while Zygisk consumes `zygisk` overrides. Only the delivery
sink differs after that. Ports use the same canonical parser and package→UID
resolver, but project `ports: true` roles into iptables rules.

```
crates/
  apatch-abi/       # read-only/load supercall command compatibility (KPM)
  protocol/         # lib, NO serde — the wire (parse/format), shared by the
                    #   Zygisk .so and core
  activator/        # library linked into core (no bins): JSON schema (serde),
                    #   pkg→uid resolution, project_native(), kmod finit_module,
                    #   KPM load/ctl0, ports iptables-restore, live-kernel identity
  core-daemon/      # the `vpnhide-core` binary: socket server, operation
                    #   journal, once-per-boot native selection, root probes
  core-rpc/         # socket protocol types (PROTOCOL_VERSION)
  core-ffi/         # cdylib loaded by the APK (UniFFI): the app's core client
  protocol-diff/    # test-only host bridge: differential C ↔ Rust parser checks
  checks/           # the native probe library the app runs in-process; core
                    #   runs its root-only probes in its own child
  checks-jni/       # cdylib — the JNI wrapper of `checks` loaded by the APK
core/module/        # the core Magisk module: service.sh (one exec line),
                    #   customize.sh (picks the ABI binary), sepolicy.rule
zygisk/             # cdylib — the injected .so. deps: protocol (+ shadowhook).
                    #   Lean: no serde, no JSON in every app process.
```

- **Why a workspace:** `cdylib` and `bin` are different crate-types, and a crate
  that is both risks pulling `serde` into the injected `.so`. The workspace keeps
  the `.so`'s dependency tree lean (protocol only) while core freely uses `serde`.
- **When it runs:** the core module's `service.sh` is a single `exec` of the
  binary; there is no other shell on the device, and core never spawns `sh`.
  That rules out `pm`, which AOSP ships as a `#!/system/bin/sh` wrapper around
  `cmd package "$@"`: core calls `cmd package …` directly. Every program core
  spawns is an ELF binary on AOSP — `cmd`, `iptables` / `ip6tables` (and their
  `-restore` forms), `ip`, `ss`, `dumpsys`, `logcat`, `bootctl`, the root
  manager's `magisk` / `ksud`, KPatch-Next's `kpatch`, and core itself
  (`observe …`). `am`, `pm`, `settings`, `input`, `svc`, `wm`, `content` and
  friends are the AOSP sh wrappers core must never call.
  At boot core selects and loads the native backend without waiting for
  PackageManager, then delivers the config once package→UID resolution is
  possible. It lists Android users first and resolves every user separately
  instead of relying on the OEM-dependent `--user all` aggregation. If any user
  scan fails, delivery stops before replacing runtime state with a partial target
  set. The app-side target picker enumerates the same way but is fail-soft: a
  failed profile scan (other than user 0, which also gets an in-process
  `getInstalledApplications(0)` backstop) shows a banner instead of blocking the
  whole app list, and Save preserves settings for packages it couldn't see.
- **Payload integrity:** core checks the selected module's payload (`*_artifact_state`
  in the root snapshot) and publishes a typed boot outcome; a missing or
  unreadable payload is a failed load, never a silent fall-through to another
  backend. KPM remediation means installing the complete `vpnhide-kpm.zip` through
  the root manager, not extracting the inner `vpnhide.kpm`.
- **KPM load preflight:** core parses only the leading numeric `major.minor` from
  `uname -r`; patchlevels and vendor suffixes do not affect the offset-table
  family. It refuses an unknown or unsupported family before invoking
  KernelPatch, and the KPM repeats the authoritative check in kernel context.

### 4.2 The native channels

| Backend | Channel | Kind | Note |
|---|---|---|---|
| kmod / built-in | `/proc/vpnhide_ctl` | proc node | write = config (kernel parses to memory); read = status+stats |
| KPM | KPM ctl0 supercall | supercall | APatch/FolkPatch direct supercall or KPatch-Next runtime `kpatch kpm ctl0`; no file |
| Zygisk | `targets.txt` in its module dir | file | the `.so` reads it via the `get_module_dir()` fd |

Why Zygisk needs a file in its module dir even though there is one canonical: the
`.so`'s **only** privileged read handle per fork is the module-dir fd Zygisk hands
it (§7). This is a *mechanism* constraint, not a format one — core simply derives
the protocol config there. (The protocol text — not JSON — keeps the injected
`.so` free of a JSON parser and carries the per-hook `hookmask` + `debug` that a
flat package list could not express.)

### 4.3 Kernel safety

The Java and native layers are orthogonal: LSPosed may run alongside the one
native backend.

The `.ko`/built-in and KPM exclusion is also a kernel safety boundary. They wrap
the same kernel functions with different mechanisms (kretprobes or source call
sites versus KernelPatch inline hooks); co-residence has hard-frozen a device.
Core's candidate scan treats a live `/proc/vpnhide_ctl` or a live KPM as a
candidate, so it never loads a second kernel backend over a live one. A KPM
loaded outside core (APatch/FolkPatch's KPM screen, KPatch-Next, an embedded
KPM, or by hand) is a supported install method: it is a live candidate like any
other, so with nothing else installed core selects and adopts it, and with
anything else installed or live it is a `Conflict`. An installed `vpnhide_kpm`
module whose KPM is already live is one candidate, not two: core adopts the
running copy and never loads the module's artifact over it. Before adopting,
core reads `KPM_INFO` (`version=`, `args=`): a version older than
`MIN_KPM_VERSION` (1.2.0, the first control-v2 release), an unrecognised version
or a failed status handshake is `KpmIncompatible`, and nothing is configured.
A runtime that cannot report module info (a `kpatch` CLI without `kpm info`)
falls back to the status handshake alone. The adopted KPM's load-time
`filesystem_hiding=1` is read back from `args=`. Loading a kernel module with
`insmod` outside core still bypasses the guard and is unsupported.

Core verifies identity on the open proc file before writing family-specific
configuration (`crates/activator/src/kernel_control.rs`): a node that reports
`0x4` is never configured as the `.ko` and vice versa, and it never calls
`finit_module` when a control node already exists. These are userspace
protections: a manual root `insmod` bypasses them.

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
      locked until the user unlocks the device, so core at **boot** (which runs
      *before* first unlock) can't read it.
    - **`/data/adb` is DE-accessible** (device-encrypted, decrypted at boot — Magisk
      reads modules from there pre-unlock), and root-only. So core (root) **can**
      read it before unlock.
- **What it unlocks: APatch boot activation.** With the key on disk, core
  reads it at boot and uses APatch's KernelPatch supercall ABI directly
  to load/configure the KPM — so APatch reaches **parity with keyless KPatch-Next**
  (protection active after a reboot, no app open needed). Some FolkPatch/APatch
  builds expose a trusted `su` token to already-authorized root callers; on those
  builds core can use that token and no saved SuperKey is needed. Without either
  token, the boot outcome waits for the SuperKey; saving it in Settings (a fixed
  core `root_effect`) makes core retry the load.
- **Plain file, not Keystore.** Core is a Rust binary, not an Android
  app, so it cannot call Android Keystore (a framework/binder API). Keystore-wrapping
  could protect the *app-time* use only, not the boot path. So the boot-usable copy
  is a plain file; FBE DE-encryption protects it at rest on a powered-off device.
- **Threat-model fit:** readable by **root only** (trusted in this project's model),
  never by unprivileged apps or non-root `adb` (§7). On APatch/FolkPatch core
  calls the supercall directly, so the key is not passed through an external
  `kpatch` argv.

---

## 7. SELinux & visibility — who can read what

Threat model: an **unprivileged** app; we do **not** hide from root. The whole layout
falls out of one fact — three reader contexts with different reach:

| Reader | Context | Can reach |
|---|---|---|
| core | root (started by the root manager) | anything |
| app | untrusted app, **no root** | its own data and core's socket (`/data/user_de/0/dev.okhsunrog.vpnhide/vpnhide-core.sock`) |
| LSPosed hook | `system_server` domain, **no root** | `system_data_file` only — **not** `/data/adb` |
| Zygisk `.so` | per-fork, module-dir **fd** only | only what that fd points at |

Consequences:

- **Canonical → `/data/system`** (`system_data_file`, `0640 root:system`): core
  (root) + LSPosed(`system_server`) read it, the app sees it through core; unprivileged apps and
  non-root `adb` get EACCES. (The one place `system_server` can read.)
- **Superkey → `/data/adb`** (`0600 root`, DE): root-only and boot-readable.
- **Zygisk → a copy in its module dir**: because its only privileged handle is the
  module-dir fd. A direct `open()` of the canonical from the zygote domain would need
  a fragile `sepolicy.rule`. So: a cheap derived copy, written by core.

Visibility of the two sensitive files, concretely:

| Reader | `/data/system/vpnhide_config.json` (0640 root:system) | `/data/adb/vpnhide/superkey` (0600 root) |
|---|---|---|
| unprivileged app | no | no |
| root process (core) | yes | yes |
| VPN Hide app | through core only | write-only, through core |
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
| `/data/adb/vpnhide/core/` | core's journal, boot decision and logs ([state.md](state.md)) | persistent, root-only |

## 9. Contract ownership

Each concern has one authoritative home; cross-links provide context without
redefining the other contracts:

| Concern | Authoritative document |
|---|---|
| Canonical JSON schema and desired state | this document (§2) |
| JSON projection, native selection, and activation policy | this document (§4); [core-2.0 architecture](notes/core-2.0-architecture.md) |
| Persistent paths, diagnostic files, lifetimes, and boot sequence | [state.md](state.md) |
| Control-v2 / telemetry-v1 bytes, registries, and runtime framing | [protocol.md](protocol.md) |
| `.ko` module parameters and hook-install behavior | [kmod README](../kmod/README.md) |
| KPM load arguments and KernelPatch runtime behavior | [KPM README](../kmod/kpm/README.md) |

LSPosed reads canonical JSON directly (§3), while core serialises control v2 for
the selected native backend (§4). The app submits desired state to core; it does
not hand-build per-channel wire payloads.


## App-side operation ownership

`CanonicalConfigRepository` owns one process-lived configuration coordinator.
Every app write goes to core as one operation; screens submit field intent
applied to a fresh canonical read. Core's operation receipt reports persistence
and native/Ports delivery as separate phases; secret and cleanup steps are fixed
core `root_effect`s with their own outcomes. Confirmed
config reaches controls before slower observation-cache refresh completes.
See [config coordinator](config-coordinator.md) for conflicts, logging ownership,
unknown-outcome recovery and first-adoption behavior. This does not change the
canonical schema, backend protocol or LSPosed direct-read contract.

Live-kernel identity checks before loading or configuring are described in §4.3.
