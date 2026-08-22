# Reading a VPN Hide debug bundle

How to open, navigate, and diagnose from the diagnostic archive the app produces
(`vpnhide_debug_*.zip`, `vpnhide_logcat_*.zip`, `vpnhide_kernel_images_*.zip`).
Written for humans **and** AI agents triaging a bug report. If you only remember
one thing: **`state.json` is the whole report**, everything else in the zip is a
heavy attachment beside it.

Source of truth for this format (read these if the doc drifts):
- [`VpnHideState.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/VpnHideState.kt) — the top-level `@Serializable` model + `buildVpnHideState` (what every field means / where it comes from).
- [`DiagnosticReport.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/DiagnosticReport.kt) — the `report` object (gate, layers, checks).
- [`CheckOutcome.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/CheckOutcome.kt) / [`LayerStatus.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/LayerStatus.kt) — outcome/status/verdict enums (the `kind` values).
- [`DebugShellSnapshot.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/DebugShellSnapshot.kt) — every raw `sections` entry + the exact shell command behind it.
- [`DebugExport.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/DebugExport.kt) / [`KernelImageExport.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/KernelImageExport.kt) — zip packaging.

Related: [diagnostics.md](diagnostics.md) (how the checks/verdicts are computed and
rendered), [state.md](state.md) (every path/proc entry the sections read),
[protocol.md](protocol.md) (the `/proc/vpnhide_ctl` wire in `kmod_state`).

---

## 1. What's in the zip

Every export is a `.zip` (forums/messengers accept `.zip`, reject bare `.json`;
JSON also compresses ~10×). Three capture kinds, told apart by
`state.json → captureKind`:

| `captureKind` | Zip entries | What triggered it |
|---|---|---|
| `debug` | `state.json` (+ optional `manifest.txt`, `kernel_partitions.txt`, `images/*.img.gz`) | Diagnostics → **Собрать отладочный лог** ("Collect debug log"). The main one. |
| `full_system_logcat` | `logcat_full.txt` + `state.json` | The full-system logcat recorder (Start/Stop). No checks run → `report`/`gate` are null; the payload is the raw log. |
| `kernel_images` | `manifest.txt` + `kernel_partitions.txt` + `images/*.img.gz` + `state.json` | The "Образ ядра" (kernel image) option — for boot/kernel-load bugs. |

`state.json` is present in **all** of them and is where you start.

### Open it

```sh
unzip -o vpnhide_debug_*.zip -d bundle
cd bundle
# top-level shape
jq 'to_entries | map({key, t: (.value|type)})' state.json
# or, no jq:
python3 -c "import json;d=json.load(open('state.json'));[print(k, type(v).__name__) for k,v in d.items()]"
```

Everything below is a `state.json` field unless noted.

---

## 2. The 30-second triage (read in this order)

1. **`captureKind`, `app.version`, `device`, `schema`** — what am I even looking at? (model, Android release, ABI, app+backend version, and which bundle format — §2.1 if it isn't the one this doc describes).
2. **`gate`** — was the run *measurable*? Only `ROUTED` yields real verdicts. `VPN_OFF` / `SELF_NOT_ROUTED` / `NEEDS_RESTART` mean "we deliberately measured nothing" — a clean-looking report there is **not** evidence of health. (See §4.)
3. **`nativeVerdict` / `javaVerdict`** — `Ok` / `Partial` / `Broken` / `null`. `null` = gated (see #2) or not measured.
4. **`rootShell`** — did the snapshot shell actually have root? If `uid != 0` or `runtimeCheckable=false`, "inactive"/"not verified" readings are unreliable, not facts. (See §6 — this is the #1 misread.)
5. **`activeBackend` + `kmodLoadStatus`** — which backend is in charge, did it load this boot, any `brokenReason`.
6. **`errors`** — non-fatal capture failures. If it lists `snapshot truncated at: <section>`, every section at/after that one is missing or partial (see §8).
7. Then drill into the raw `sections` and logs for the specific symptom (§5, §7, §9 playbooks).

### 2.1 Schema versions

`state.json → schema` says which shape the bundle was written in. **This doc
describes schema 2.** One number covers the whole bundle; there is no per-object
version.

| Schema | Shipped in | What changed |
|---|---|---|
| 2 | unreleased | Sealed `kind` discriminators are compact snake_case everywhere. Previously `dashboard.lsposed` and `dashboard.protection` carried fully-qualified class names (`dev.okhsunrog.vpnhide.LsposedState.Active`) instead of `active`/`blocked`. The separate `report.schema` field is gone — the top-level one is the only version. |
| 1 | 1.0.0 – 1.2.5 | Initial format. |

Rules for changing it (enforced by `BundleSchemaGoldenTest`, which pins the
serialized shape against `app/src/test/resources/bundle/state_golden.json`):

- A field removed, renamed, or given a new meaning → **bump** `VPNHIDE_STATE_SCHEMA`
  and add a row above.
- A field added → no bump; a reader of an older bundle just finds it missing.
- Either way the golden file has to be refreshed
  (`UPDATE_GOLDEN=1 ./gradlew :app:testDebugUnitTest --tests '*BundleSchemaGoldenTest*'`),
  so no shape change reaches a release unnoticed.

Bundles are read, not machine-parsed, so nothing rejects an old schema — the
number exists to tell you *which* doc revision applies to the file in front of
you.

---

## 3. Top-level `state.json` fields

Derived (typed) fields first — these are the *same domain objects the app UI
renders*, so the bundle can't disagree with what the user saw on screen.

| Field | Meaning |
|---|---|
| `schema` | Bundle schema version — **this doc describes 2** (§2.1). |
| `generatedAt` | ISO-8601 capture time. |
| `captureKind` | `debug` / `full_system_logcat` / `kernel_images` (§1). |
| `app` | `{packageName, version}` — `version` is `"1.2.5 (10205)"` (name + versionCode). |
| `device` | `{manufacturer, model, androidRelease, sdk, abis}`. |
| `selfNeedsRestart` | The picker app added itself as a target but its own hooks aren't applied to this process yet → checks can't measure; user must reboot. |
| `gate` | `DiagnosticGate` — see §4. `null` for logcat/kernel captures (no check run). |
| `nativeVerdict` / `javaVerdict` | `Ok`/`Partial`/`Broken`, or `null` if not a measured (`ROUTED`) run. |
| `report` | The full `DiagnosticReport` (§4). `null` when no checks ran. |
| `backends` | Per-module state for `kmod` / `kpm` / `zygisk` (installed? active? version? `brokenReason`? `pendingReboot`?). |
| `activeBackend` | The **one** native backend in charge (`id` + `state`). Priority kmod > KPM > zygisk. |
| `ports` | The portshide module `ModuleState` (localhost port blocker). |
| `kmodLoadStatus` | Boot-time `.ko` load result (uname, kprobes/kretprobes ok, insmod exit, dmesg tail, `filesystemHiding`). The richest single "did the kernel module come up" field. |
| `dashboard` | Full live dashboard model. **`null` in file exports** (only the agent-bridge `getState` fills it). |
| `config` | The canonical desired-state config as structured JSON (`/data/system/vpnhide_config.json`) — answers "is app X even a target, with which roles". |
| `statistics` | Point-in-time hook-counter totals (per-uid/per-method hit counts). Always present in a debug export, even without forensics. |
| `rootShell` | Snapshot-shell self-diagnosis (§6). The "not verified vs inactive" discriminator. |
| `sections` | The raw shell-probe map — ground truth the typed fields were derived from + all forensic captures. **Empty unless forensics was on** (§5). |
| `dmesg` / `logcat` | Kernel ring buffer / app-tag logcat captured during the run (forensics only). |
| `bootLsposedLogcat` | Best-effort boot-time LSPosed/Vector lines from the current ring buffer (may have rotated away). |
| `lsposedConfigDb` | LSPosed module enable state + scope for this app. |
| `hookReport` | Native hook install mask + counter deltas across the forced check run (§7). |
| `debugCapture` | Whether debug logging was force-enabled for this capture (`forced`) and the toggle exit codes. |
| `captureOptions` | `{forensics, appList}` — self-documents what was included. |
| `errors` | Non-fatal capture failures (a probe that threw, a truncated section). Partial data is flagged here, never silently dropped. |

---

## 4. The diagnostic report — is the VPN actually hidden?

`report` is the single canonical verdict object; the dashboard, Diagnostics
screen and this bundle are all pure renders of it. *Computed in
[`DiagnosticReport.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/DiagnosticReport.kt)
(`buildDiagnosticReport`); outcome classification in
[`CheckOutcome.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/CheckOutcome.kt)
+ [`GroundTruthProbe.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/GroundTruthProbe.kt);
the per-vector spec + hook coverage in
[`NativeChecks.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/NativeChecks.kt).*

### `gate` gates everything

`report.gate` (and top-level `gate`) is one of, most-blocking first:

| Gate | Meaning | Verdicts? |
|---|---|---|
| `NEEDS_RESTART` | This app was just added as a target; its hooks aren't live in this process yet. | none |
| `VPN_OFF` | No VPN interface up — nothing to hide. | none |
| `SELF_NOT_ROUTED` | VPN up, but this app is split-tunnelled **out** of it, so it can't observe the tunnel. | none |
| `ROUTED` | Measured. **Only here are `nativeVerdict`/`javaVerdict` meaningful.** | yes |

A bundle with `gate != ROUTED` and null verdicts is **not** a problem by itself —
it means the capture was taken when nothing could be measured. If a user reports
"diagnostics look empty", check the gate first; the fix is usually "turn the VPN
on / route this app through it, then recapture."

### Layers, status, checks

`report.native` and `report.java` are each a `LayerReport`:

- `status.kind`:
  - `active {hidden, leaks}` — measured. `hidden` = vectors the backend provably hid (root differential); `leaks` = owned vectors that still leaked.
  - `inactive` — installed but not loaded this boot (reboot/toggle needed).
  - `absent` — no module for this layer.
  - `unverified` — installed but the shell couldn't confirm liveness (see §6) → rendered "not verified", never a false "inactive".
- `unownedLeaks` (native only) — leaks on vectors this backend doesn't own; a warning, not counted against the verdict.
- `checks[]` — one `DiagnosticCheck` per vector: `{id, label, layer, outcome, appDetail, groundTruthDetail, expectedHooks, owned}`.

**`Verdict`** (only when `ROUTED`): `Ok` = nothing owned leaks · `Partial` = hides
some but an owned vector still leaks · `Broken` = active yet hid nothing while
leaking (loaded but dead).

**`CheckOutcome.kind`** per check:

| `kind` | Meaning |
|---|---|
| `leak` | App saw VPN-shaped data on this vector — a real leak. |
| `hidden_backend` | App saw nothing, root did → the active backend hid it. The good case. |
| `hidden_selinux` | The app's probe was EACCES-blocked by SELinux, not a backend hook. Inconclusive for backend health. |
| `nothing_to_leak` | Nothing VPN-shaped for anyone (root saw nothing either). |
| `not_measured` | Probe produced no usable observation (`reason`: no network permission / no ground truth). |

To list every leaking vector fast:

```sh
jq -r '.report | (.native,.java) | .checks[] | select(.outcome.kind=="leak")
       | "\(.layer) \(.id) \(.label): \(.appDetail)"' state.json
```

---

## 5. The `sections` map — raw ground truth

`sections` is a `name → text` map of raw root-shell probe output. It is the
evidence the typed fields were derived from. **Only populated when forensics is
on** (the "Расширенный лог" toggle, default on for bug reports); a lean export has
`sections: {}`. Two collectors are merged into it — the authoritative root
snapshot (used to derive the typed state) plus the forensic batch (network,
proc/net, kernel symbols, inventory) — with the root snapshot winning on key
collisions.

Each entry is framed in the batch script by `emit_cmd`/`emit_file`/`emit_eval`;
a missing file shows `(missing: <path>)`, a failed command shows its stderr
inline (the script never aborts on one probe). *Every section name below maps 1:1
to an `emit_*` line in
[`DebugShellSnapshot.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/DebugShellSnapshot.kt)
— open it to see the exact command; the `$UPPER_CASE` paths resolve in
[`ConfigChannels.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/ConfigChannels.kt).*

### By purpose

**Environment / kernel**
- `uname`, `proc_version`, `proc_cmdline` (bootloader args; serialno/MACs redacted), `getprop_selected` (fingerprints, verified-boot, slot), `kernel_release`.
- `kernel_config` — the KPROBES/MODULES/SELINUX/NETFILTER config flags that gate whether a backend can even work.
- `selinux`, `getenforce` — enforcing? security contexts on `/data/adb`, `/proc/net`, `/sys/class/net`.
- `root_manager` — Magisk/KSU/APatch presence + versions + `/data/adb` perms.

**Native kernel backend (.ko)**
- `kmod_prop`, `kmod_module_state` (file flags, `.ko` + activator sha256, staged-for-reboot state), `kmod_load_status`, `kmod_load_dmesg` (the boot load log — kretprobe registration lines), `kmod_state` (**live `/proc/vpnhide_ctl`** telemetry: installed-hook mask, per-uid counters — see [protocol.md](protocol.md)).
- `proc_modules` (is `vpnhide*` in `/proc/modules`), `kprobes` (`/sys/kernel/debug/kprobes/list` for our symbols), `kernel_symbols` (kallsyms presence of each hooked function — key for "symbol missing on trimmed OEM KMI").

**KPM backend** — `kpm_prop`, `kpm_module_state`, `kpm_load_status`, `kpm_state` (activator `state` dump), `kpatch_runtime` (APatch/KPatch presence, `kpatch hello`/`kpm list`).

**Zygisk backend** — `zygisk_prop`, `zygisk_module_state`, `zygisk_status`, `zygisk_runtime`.

**Ports (localhost blocker)** — `ports_prop`, `ports_module_state`, `ports_load_status`, `ports_load_log`, `ports_state` (**live `iptables`/`ip6tables` `vpnhide_out` chain** dump).

**LSPosed / Java layer** — `lsposed_state`, `lsposed_framework` (which framework: LSPosed/Vector), `lsposed_files` (config DB perms).

**Config / targets** — `canonical_config` (the desired-state JSON, mirrored into top-level `config`). `legacy_kmod_targets` / `legacy_kpm_targets` / `legacy_zygisk_targets` / `legacy_lsposed_targets` / `legacy_ports_observers` / `legacy_hidden_pkgs` / `legacy_observer_uids` — the pre-1.0 config files ([state.md §1](state.md)). Non-empty only on an install that upgraded across 1.2.0 without importing yet: the import deletes them, so a bundle showing both these and a populated `canonical_config` means the user was offered the merge/replace banner and hasn't answered.

**App enumeration** — `app_scan_diagnostics` (privacy-safe: per-user exit code + package counts + first stderr line, **no names/paths** — diagnoses "couldn't read all profiles" / the ARG_MAX overflow). `pm_users` / `pm_packages` carry the real names/paths and are **redacted out unless `appList` was opted in** (§10).

**Network (forensic)** — `network_addr` (`ip -d addr`), `network_operstate`, `network_routes` (`ip route show table all`), `network_rules` (`ip rule`), `network_sockets` (listening localhost sockets), `connectivity_dump` (`dumpsys connectivity`, VPN/tun/agent lines), `proc_net_route` / `proc_net_ipv6_route` / `proc_net_if_inet6` / `proc_net_tcp[6]` / `proc_net_udp[6]` / `proc_net_dev` / `proc_net_fib_trie`.
- `vpn_ifaces` — `/sys/class/net/*/operstate` for every interface. The authoritative interface list; the app's `isVpnActive` reads it.

**Module presence flags** (one-liners, from the root snapshot) — `kmod_module_dir`, `*_activator_state`, `*_disabled`, `*_pending_update`, `superkey_saved`, `proc_exists` (does `/proc/vpnhide_ctl` exist), `ports_chain` (does the iptables chain exist), `snapshot_shell_uid` (backs `rootShell`, §6), `module_inventory` (dump of every relevant `/data/adb/modules[_update]/*`).

### Fast lookups

```sh
# dump one section
jq -r '.sections.kmod_load_dmesg' state.json
# list section names + sizes
jq -r '.sections | to_entries | map("\(.key)\t\(.value|length)")[]' state.json
# grep across all sections at once
jq -r '.sections | to_entries[] | "== \(.key) ==\n\(.value)"' state.json | grep -n -i tun0
```

---

## 6. `rootShell` — the reading you must not misread

The single most common misdiagnosis is treating an "inactive"/"not verified"
module as *proven off*. `/proc/vpnhide_ctl` is `0600` and the iptables chain
needs root; if the snapshot shell wasn't uid 0, a negative liveness read is
**meaningless**, not a fact.

```json
"rootShell": { "uid": 0, "idLine": "...context=u:r:ksu:s0", "context": "u:r:ksu:s0",
               "errnoCtl": "ok", "runtimeCheckable": true }
```

- `uid != 0` **or** `runtimeCheckable=false` → treat every "inactive"/"absent"
  runtime reading with suspicion; the layer status will correctly say
  `unverified` rather than `inactive`. Cross-check `errnoCtl` (`eacces` = perms,
  `enoent` = genuinely not present).
- `uid == 0`, `errnoCtl=ok`, `runtimeCheckable=true` → liveness readings are
  trustworthy; an "inactive" here is real.

---

## 7. Logs and hook counters

- **`dmesg`** — captured after a `dmesg -c` clear at run start, so it's mostly
  *this run's* kernel output. During a debug export the app **force-enables
  kernel debug** (`debugCapture.forced`), so `vpnhide:` trace lines
  (`sk_setsockopt_entry … action=…`, `dev_ioctl_entry …`, `config applied …
  debug=1`) appear here — invaluable for "is a hook firing / denying". Note the
  ring buffer is finite; on a chatty device early-boot lines may have rotated.
- **`logcat`** — only VPN Hide's own tags (`VPNHideTest`, `VpnHide-*`,
  `vpnhide*`, `shadowhook_tag`, …). The app's per-check PASS/FAIL lines land here.
- **`bootLsposedLogcat`** — best-effort boot-time LSPosed/Vector attach lines
  (`cs_*` telemetry, hook install). May be empty if the buffer rotated. See
  [lsposed-hook-debugging.md](lsposed-hook-debugging.md).
- **`hookReport`** — installed-hook mask + per-counter deltas measured across the
  forced check run (the SOCKET_BIND_INTERFACE deny-count etc.) — tells a dead
  redirect from a working one.

---

## 8. `errors` and truncation — trust but verify completeness

`errors` lists non-fatal failures; the document always serializes, partial data
is flagged rather than lost. Common entries:

- `debug shell exit=<n>` — the batched root script returned non-zero (often just
  one probe failing; not necessarily fatal).
- `snapshot truncated at: <section>` — the batch overran the 60s su timeout. The
  named section is cut mid-output (its body ends with a `(TRUNCATED: …)` marker),
  and **every section that would come after it is absent**. `sections` also gets
  a `debug_snapshot_truncated: <section>` marker.

The section order in [`DebugShellSnapshot.kt`](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/DebugShellSnapshot.kt)
is the emit order — if truncation hit `app_scan_diagnostics`, the `network_*` /
`proc_net_*` sections after it won't be present. Don't read "section missing" as
"feature absent" without checking `errors` / `debug_snapshot_truncated` first.

---

## 9. Diagnostic playbooks

**"Kernel module won't load / backend inactive"**
1. `kmodLoadStatus` — `loaded`, `insmodExit`, `insmodStderr`, `unameR`,
   `kprobes`/`kretprobes`, `dmesgTail` (look for `Unknown symbol` / `register_kretprobe failed`).
2. `sections.kmod_load_dmesg` for the full boot load log.
3. `sections.kernel_symbols` — is a hooked symbol missing from kallsyms (trimmed OEM KMI)?
4. `sections.kernel_config` — `CONFIG_KPROBES`/`KRETPROBES`/`MODULES` present?
5. `backends.kmod.brokenReason`, `pendingReboot`; `rootShell` (is "inactive" even trustworthy — §6).

**"A vector leaks / VPN detectable"**
1. Confirm `gate == ROUTED` (else not measured — §4).
2. `report.native.checks[] where outcome.kind == leak` → the vector + `expectedHooks` + `owned`.
3. `hidden_selinux` outcomes are inconclusive, not wins.
4. `dmesg` (debug on) — is the expected hook firing for the vector's syscall?
5. Cross-check the raw section for that vector (e.g. `proc_net_route`, `network_addr`).

**"No network at all / DNS broken with the module"**
1. `vpn_ifaces` + `network_addr` + `proc_net_dev` — which interfaces exist; any physical/default iface being hidden?
2. `dmesg` (debug on) — hook actions; `sk_setsockopt_entry action=deny`, and `active_hook_mask`.
3. `ports_state` — is the iptables `vpnhide_out` chain over-broad?
4. `report`/`gate`, `statistics` (per-hook hit counts).

**"App list won't load / demands unlock profile"**
1. `sections.app_scan_diagnostics` — per-user `exit`, `package_lines`, `with_uid`, `with_path`, `stderr` (which user fails and how).
2. `device` (MIUI/HyperOS?), `errors` (ARG_MAX-era failures).

**"Ports / localhost blocking wrong"** — `ports`, `ports_load_status`,
`ports_load_log`, `ports_state` (live chain), `config` (which apps have the
`ports` role).

**"Java/LSPosed hooks not attaching"** — `report.java`, `lsposedConfigDb`,
`sections.lsposed_state`, `bootLsposedLogcat`, `hookReport`; see
[lsposed-hook-debugging.md](lsposed-hook-debugging.md).

---

## 10. Privacy / redaction (what's deliberately omitted)

- `pm_packages` and `pm_users` (full installed-app list with paths+UIDs, profile
  names) are **redacted out of `sections` unless the user opted in** via the
  app-list toggle (`captureOptions.appList=true`). The privacy-safe
  `app_scan_diagnostics` (counts only, names redacted) always stays.
- `proc_cmdline` has `androidboot.serialno` / `wifi_macaddr` / `btmacaddr`
  scrubbed.
- A lean (non-forensics) export carries **no `sections` and no logs** — just the
  typed state. Check `captureOptions.forensics` before concluding "the section
  isn't there."

---

## 11. Kernel image bundle

When the kernel-image option is on, the zip also carries the boot-chain
partitions for offline inspection (unpacking the `.ko`'s target kernel, checking
KMI, etc.):

- `manifest.txt` — `schema`, `slot_suffix`, which partitions were `included` /
  `skipped`, and per-partition `present`/`path`/`size_bytes`/`dd` stats +
  sha256 of the gzipped image.
- `kernel_partitions.txt` — richer per-partition metadata (real block device,
  symlink target, verified-boot state, sha256 of the raw partition).
- `images/*.img.gz` — the gzipped partition images themselves (`boot_a`,
  `init_boot_a`, `vendor_boot_a`, `vendor_kernel_boot_a`, …). Large.

---

## 12. Code map — where each part is produced

Which source file to open to understand (and trace the origin of) each part of
the bundle. All paths are under
`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/` unless noted.

| Bundle area | Produced / derived in | Read it to understand |
|---|---|---|
| The whole `state.json` model + how a capture is folded together | `VpnHideState.kt` (`buildVpnHideState`) | every field's meaning, provenance, and the redaction rules |
| Zip packaging, forensics/appList gating, dmesg/logcat capture | `DebugExport.kt` (`exportDebug`/`buildDebugState`) | how the bundle is assembled and what "forensics" actually turns on |
| `report`, `gate`, layers, checks, verdicts | `DiagnosticReport.kt` (`buildDiagnosticReport`) | how the verdict is computed **once** and rendered everywhere |
| `CheckOutcome.kind` (leak/hidden_backend/hidden_selinux/…) | `CheckOutcome.kt` + `GroundTruthProbe.kt` | how the root-differential decides who hid the VPN |
| `LayerStatus` + `Verdict` (Ok/Partial/Broken) | `LayerStatus.kt` | presence-before-checks, and the exact verdict rules |
| Native check specs (ids, `expectedHooks`, coverage) | `NativeChecks.kt` (`NATIVE_CHECKS`) | what each `report.native.checks[].id` probes and which hooks own it |
| Running the probes (native + app-side + Java) | `checks/NativeProbe.kt`, `JavaChecks.kt` (`runAllChecks`) | how `appDetail` / observations are produced |
| `backends`, `activeBackend` (detection + kmod>KPM>zygisk priority) | `NativeBackendData.kt` | how installed/active/broken/pendingReboot is decided per module |
| `ports`, `kmodLoadStatus` | `DashboardData.kt` (`detectPortsModule`, `readKmodLoadStatus`) | how the ports module + boot `.ko` load result are parsed |
| Optional native hooks (filesystem-hiding coverage) | `FilesystemHidingData.kt` | which optional hooks the active backend installed |
| `statistics` (per-uid / per-method counters) | `StatisticsData.kt` (`buildStatisticsState`) | how the hook-counter totals are built |
| `hookReport` | `HookDiagnostics.kt` | installed-hook mask + counter deltas across the run |
| `rootShell` (not-verified vs inactive) | `VpnHideState.kt` (`RootShellDiag`) ← `snapshot_shell_uid` probe | why a negative liveness read may be untrustworthy |
| The raw `sections` batch — every probe + its exact shell command + emit order | `DebugShellSnapshot.kt` | what each section literally runs (and the truncation model) |
| The path constants in those commands (`$PROC_CTL`, `$KMOD_MODULE_DIR`, …) | `ConfigChannels.kt` | which files/procs each section reads (also [state.md](state.md)) |
| The authoritative root snapshot behind the typed fields | `RootSnapshotCache.kt` | which sections are root-authoritative (win key collisions) |
| Kernel-image bundle (`manifest.txt`, `kernel_partitions.txt`, `images/`) | `KernelImageExport.kt` | how partitions are dumped, gzipped, hashed |
| `sections.kmod_state` = the live `/proc/vpnhide_ctl` wire | `kmod/vpnhide_kmod.c` (`ctl_seq_show`) + [protocol.md](protocol.md) | the telemetry wire (status + per-uid stats) the section dumps |

Rule of thumb: a **typed** top-level field → find where it's *derived*
(`*Data.kt` / `DiagnosticReport.kt`); a **raw `sections` entry** → find the
`emit_*` line in `DebugShellSnapshot.kt` for the exact command, then
`ConfigChannels.kt` / [state.md](state.md) for what that path is.

---

## Quick reference — "where does X live?"

| Question | Look at |
|---|---|
| Which backend is active? | `activeBackend.id`, `backends` |
| Did the `.ko` load this boot? | `kmodLoadStatus`, `sections.kmod_load_dmesg` |
| Is the VPN actually hidden? | `report` + `nativeVerdict`/`javaVerdict` (requires `gate==ROUTED`) |
| Which vector leaks? | `report.*.checks[] .outcome.kind=="leak"` |
| Is a hook firing? | `dmesg` (`vpnhide:` traces), `hookReport`, `sections.kmod_state` |
| Is a kallsyms symbol missing? | `sections.kernel_symbols` |
| Is an app a target, with which roles? | `config.apps` |
| Live iptables chain? | `sections.ports_state` |
| Interfaces on the device? | `sections.vpn_ifaces`, `sections.network_addr` |
| Can I trust an "inactive" reading? | `rootShell` (§6) |
| Was the capture even measurable? | `gate` (§4) |
| Is anything missing/cut off? | `errors`, `sections.debug_snapshot_truncated` |
| What was included in this capture? | `captureOptions`, `captureKind` |
