# Reading a VPN Hide debug bundle

Start with `state.json` inside the exported ZIP. This guide describes the current
schema11, paired APK/Core RPC31 and diagnostic contract9. Historical captures use
[the versioned reference](archive/2026-11-03-debug-bundle-legacy-reference.md).
Current exports do not require a foreign VPN or split-tunnel membership, and
collecting a file does not admit a diagnostic run.

## 1. What's in the zip

| `captureKind` | Contents and owner |
| --- | --- |
| `debug` | `state.json`, collection status, optional Java files and kernel images. APK adapters currently orchestrate capture and packaging; Core performs privileged acquisition. |
| `full_system_logcat` | `state.json` and `logcat_full.txt`. Core owns the bounded recording process, lease, durable session, stop and delivery/ACK lifecycle. |
| `kernel_images` | `state.json`, manifest and partition/image entries from the explicit kernel-image acquisition. This is separate from normal state reads. |

`collection-plan.txt`, `java/collection.txt` and optional `kernel/collection.txt`
explain requested/skipped/failed attachments. Java evidence can include
`java-manifest.json`; inspect its per-file freshness and omissions. A failed
optional attachment can leave a useful baseline archive. File creation alone is
not diagnostic success.

```sh
unzip -l vpnhide_debug_example.zip
unzip -p vpnhide_debug_example.zip state.json > state.json
jq '{schema, generatedAt, captureKind, captureOptions, errors}' state.json
```

## 2. The 30-second triage

1. Read `schema`, `app`, `device`, `captureOptions` and `errors`.
2. Read `autonomousDiagnostics.availability`, `current`, source identities and
   `rootJoined`. Capture time is not measurement time.
3. Read `autonomousDiagnostics.assessment`: required, proven, leaked, unverified,
   controls, reuse blockers and infrastructure/cleanup facts.
4. Read `nativeStatus`: selection, attempts, delivery, observed runtime and setup
   are separate. Neither an installed module nor a setup-ready state proves hiding.
5. Inspect the relevant retained run evidence, then raw `sections`, logs and
   counters. A manual process observation cannot refresh an older game process.

### 2.1 Schema versions

One schema number covers the exported document. The golden serialization test
pins shape and stable discriminators. RPC, diagnostic, private journal and kernel
wire versions are separate boundaries.

| Schema | Shipped in | What changed |
|---|---|---|
| 11 | main (unreleased) | Logcat duration now includes suspend using CLOCK_BOOTTIME. `clock_unavailable` is a distinct auto-stop reason; its duration can be the last measured lower bound. Field names remain stable. |
| 10 | main (unreleased) | `run.filesystem` preserves eighteen separate libc/direct-syscall stat/access/open-directory checks for sysfs and both proc-sys families, plus matching RAW observations. Denied/unsupported lookups remain unverified; late control failures retain qualified leaks. Directory enumeration remains separate. |
| 9 | main (unreleased) | `run.sysInterfaces` retains separate libc/direct-getdents64 directory proof, leaks and controls for sysfs and both proc-sys families. All six require positive same-worker RAW and ordinary-object controls; this does not complete stat/access/open or promote a denied read to proof. Shared RAW receipts retain the corresponding actual observations. |
| 8 | main (unreleased) | Native proof requires a correlated app RAW interval and confirmed revocation; privileged root controls alone remain unverified. `run.nativeRaw` preserves the full shared Rust receipt, including failed-scope observations and exact thread identity. Native evidence is retained before framework collection, so worker death cannot erase an already reported leak. |
| 7 | main (unreleased) | `run.procInterfaces` retains per-surface libc/direct-syscall proof, leaks and controls for `/proc/net/dev` and `/proc/net/if_inet6`. A denied read is unverified; proc leaks remain distinct from proven address ioctls/netlink. |
| 6 | main (unreleased) | Autonomous run exports keep separate address actor/vector proof, leaks and controls. Interface-only getifaddrs evidence no longer proves IPv4/IPv6 address coverage; grouped address requirements retain missing families and syscall actors as unverified. This is scoped evidence, not whole-device acceptance. |
| 5 | main (unreleased) | Core-owned `nativeStatus` records chosen/bundled provenance, attempts, delivery, observed runtime, desired hook coverage and setup separately. Old Kotlin backend priority/classifiers and VPN-dependent report/gate owners are removed. Dashboard omits native install recommendation/partial-hook/installed-optional projections; statistics carry source identity/currentness. Legacy nullable output slots stay null. |
| 4 | main (unreleased) | Dashboard, Diagnostics, agent and captures consume retained autonomous Core evidence. `autonomousDiagnostics` identifies the accepted Core/config/root sources, required/proven/leaked/unverified vectors, controls, cleanup and reuse blockers. Legacy gate/report/diagnostics/verdict fields remain nullable for old readers; new captures do not fabricate an app-process self-test. |
| 3 | dev-2.x (unreleased) | Measurement summaries and snapshots separate desired config revision, delivery identity and whether delivery was confirmed. An observed passing suite under uncertain delivery cannot establish current success; the UI shows application uncertainty while retaining its measured results. |
| 2 | 1.3.0 | Sealed `kind` discriminators are compact snake_case everywhere. Previously `dashboard.lsposed` and `dashboard.protection` carried fully-qualified class names (`dev.okhsunrog.vpnhide.LsposedState.Active`) instead of `active`/`blocked`. The separate `report.schema` field is gone — the top-level one is the only version. |
| 1 | 1.0.0 – 1.2.5 | Initial format. |

Rules for changing it (enforced by `BundleSchemaGoldenTest`, which pins the
serialized shape against `app/src/test/resources/bundle/state_golden.json`):

- An exported serializable model change → **bump** `VPNHIDE_STATE_SCHEMA`
  and add a row above, preserving the previous schema's documented meaning.
- Either way the golden file has to be refreshed
  (`UPDATE_GOLDEN=1 ./gradlew :app:testDebugUnitTest --tests '*BundleSchemaGoldenTest*'`),
  so no shape change reaches a release unnoticed.

Bundles are read, not machine-parsed, so nothing rejects an old schema — the
number exists to tell you *which* doc revision applies to the file in front of
you.

---


### 2.2 Autonomous evidence

`autonomousDiagnostics` is one accepted Core/config/root publication. It records
Core epoch, config revision, publication sequence, connection generation, root
observation/generation and join/currentness qualifications. A retained report can
remain useful while stale or disconnected; its freshness must not be inferred
from `generatedAt`, a new root read or unrelated publication sequence changes.

`run` carries session, diagnostic contract, fingerprint, loaded-generation and
platform evidence, stale state and optional framework attribution. The assessment
reports scope honestly; current code has no whole-suite success variant.

| Evidence | Meaning and limits |
| --- | --- |
| `run.nativeRaw` | Full shared receipt: exact worker thread/process identity, paired or failed interval and retained observations. Root visibility cannot substitute for same-worker RAW visibility. |
| `run.binding` | Separate libc/syscall actors, setter/readback and ordinary controls. A setter error cannot erase a positively observed binding leak. |
| `run.addresses` | Per-actor/per-vector IPv4/IPv6 and NULL-buffer controls. Interface-only getifaddrs proof cannot establish address coverage. |
| `run.procInterfaces` | `/proc/net/dev` and `/proc/net/if_inet6` through both read actors. Current reference `.ko` QEMU still exposes all four leaks. |
| `run.filesystem` | Eighteen optional path-lookup checks, with actor, surface, operation, proof, leaks and controls. `ENOENT` is absence; permission denial and unsupported syscalls are errors. |
| `run.sysInterfaces` | Six optional sysfs/proc-sys directory checks. Separate from the stat/access/open group. |
| `run.frameworkAttribution` | Worker and system_server process births, boot, hook-source and fixture-profile identity. A fresh worker does not prove old target processes updated. |

Unavailable stimulus, empty denied APIs, a clean Java answer or an installed-hook
mask cannot remove required checks. Qualified leaks survive later control/cleanup
failures; successful proof requires all corresponding controls. Required plans
come from the supported contract and enabled features. See
[diagnostics](diagnostics.md) for implementation and remaining code/device gates.

```sh
jq '.autonomousDiagnostics | {availability, current, rootJoined, run, assessment}' state.json
jq '.autonomousDiagnostics.assessment | {required, proven, leaked, unverified,
    nativeControls, frameworkControls, reuseBlockers, cleanupConfirmed}' state.json
```

### Native setup and statistics

`nativeStatus` retains Core currentness/source identity and its typed model:
selected implementation/provenance, attempts, delivery receipt, applied identity,
observed runtime, required/installed/missing hooks, setup and filesystem state.
These facts must stay distinct. Legacy module directories in `backends` are
inventory evidence; bundled alternatives are not independently active modules.
`activeBackend` is a Core-derived display projection, not an APK priority selector.

`statistics` is a point-in-time counter snapshot with source qualifications.
Unsigned counts can exceed signed `Long`; decimal strings retain their bits.
Capture/agent deltas and forensic reports use the shared Rust numeric comparator.
A decrease is discontinuous, not proof of a specific restart or inferred wrap.
Capture baseline/source attribution and remaining orchestration still need work.

## 3. Top-level state fields

| Field | How to read it |
| --- | --- |
| `generatedAt`, `captureKind`, `app`, `device` | Capture metadata; not measurement freshness or backend admission. |
| `autonomousDiagnostics`, `nativeStatus` | Accepted retained evidence and native setup/read model, with independent sources. |
| `dashboard`, `beforeCapture` | Presentation and earlier accepted process state. Earlier and later observations need not be the same snapshot. |
| `config` | Accepted desired canonical policy when available. Persistence, delivery and observed behavior remain separate. |
| `backends`, `ports`, `activeBackend` | Inventory/observation and display projections. Unjoined runtime data is not certified current. |
| `statistics`, `hookReport` | Counters and manual forensic interval; neither admits or completes a suite. |
| `sections`, logs, `lsposedConfigDb` | Optional raw forensics. Capture options and errors explain omissions. |
| `networkView` | Manual public Android observations of the collecting app process. It cannot promote retained Core evidence. |
| `debugCapture` | Bounded debug-logging lease/acquisition/restoration evidence. User debug intent is separate. |
| `errors`, `captureOptions`, `rootShell` | Collection limits, selected content and root-read qualifications. |

`gate`, `selfTestRunId`, `diagnostics`, `report`, `nativeVerdict`, `javaVerdict` and
`kmodLoadStatus` are historical compatibility slots, null in current captures.
Their presence in the JSON is not a surviving VPN readiness gate or verdict owner.

## 4. The diagnostic report — is the VPN actually hidden?

Read `autonomousDiagnostics.assessment` and its provenance. `leaked` records
qualified observations; `unverified` and control failures identify missing proof.
`proven` applies only to its required controlled group. It is not a general promise
about every process, API, old game instance or vendor kernel.

Do not ask the user to enable a foreign VPN or route this APK through one to make
Dashboard/diagnostics ready. Do not interpret a successful export, hidden directory,
worker completion or setup-ready backend as whole protection PASS. The retired
uncontrolled `probe checks` classifier is no longer a supported native verdict path.

## 5. The sections map — raw observations

`sections` merges fixed Core root/forensic probes; the authoritative root snapshot
wins name collisions. These are Rust collectors and fixed direct-exec commands,
not an app shell or arbitrary root command endpoint. Forensics off yields an empty
map. Framing, absent files, command exits and truncation markers are evidence.

| Group | Useful entries |
| --- | --- |
| Core/ownership | `core_prop`, `core_module_state`, `core_boot_status`, `core_log`, `core_native_boot` |
| Kernel backend | `kmod_prop`, `kmod_state`, `kmod_diag`, `proc_modules`, `kprobes`, `kernel_symbols`, `kernel_config` |
| KPM/Zygisk | `kpm_state`, `kpatch_runtime`, `zygisk_status`, associated module/artifact metadata |
| Ports | `ports_state`, `ports_chain`, typed Core per-family apply evidence |
| Java layer | `lsposed_state`, `lsposed_framework`, `lsposed_files`, boot logcat and hook report |
| Configuration | `canonical_config` and scoped legacy inventory. Migration preserves original sources; old files alone do not prove a pending import. |
| Network forensics | `network_addr`, `network_routes`, `network_rules`, `network_operstate`, `connectivity_dump`, `proc_net_*` |
| Root/package inventory | `snapshot_shell_uid`, `app_scan_diagnostics`, and explicitly included `pm_packages`/`pm_users` |

Builtin telemetry uses `kmod_state` with backend identity4; a historical
`vpnhide_builtin` directory is not a separate current builtin runtime. Read the
current [state map](state.md) and collector source for exact paths and lifetimes.

```sh
jq -r '.sections.core_native_boot' state.json
jq -r '.sections | to_entries[] | "\(.key)\t\(.value|length)"' state.json
jq -r '.sections.proc_net_dev' state.json
```

## 6. rootShell — the reading you must not misread

`rootShell.shellUid` and `runtimeCheckable` qualify the captured root read. A
non-root/unavailable observation cannot establish a backend is inactive. UID0
alone does not prove every required capability or SELinux permission. Use actual
probe exits, typed Core readiness and `errors`; never mask a failed root snapshot
as an empty successful state.

## 7. Logs and hook counters

`dmesg`, app-tag logcat, boot LSPosed logcat, hook telemetry and `hookReport` answer
different questions. A hit counter shows the hook ran; it does not by itself prove
correct filtering. Installed masks and misses are installation evidence, not the
required diagnostic plan. For Java attachment details see
[LSPosed hook debugging](lsposed-hook-debugging.md).

Core owns the full-system logcat process, caps of 64 MiB and 30 minutes, private file,
restoration and durable undelivered state. The APK asks Start/Stop, saves the copy
and acknowledges or explicitly discards it. App death does not leave an unlimited
recording. Do not introduce a second recorder while migrating export orchestration.

## 8. Errors and truncation

Read all `errors`, per-section exit/truncation markers and collection manifests.
Missing optional artifacts need not destroy a useful baseline archive. Conversely,
an archive being written does not establish all requested evidence was collected.
Late failures do not erase retained native leaks. Disconnected or unjoined data
keeps its qualifications rather than borrowing a newer observation's identity.

## 9. Diagnostic playbooks

- **Backend not ready:** inspect typed native selection, attempt and delivery;
  then boot status, loader stderr, symbols/config and actual runtime. Diagnose
  uncertainty separately from a confirmed no-effect failure; do not blindly reload.
- **Detection/leak:** inspect retained required/leaked/unverified groups and RAW,
  ordinary/end/cleanup controls. Correlate the source, actor, fixture and process
  generation before looking at forensic proc/net or Java observations.
- **Broken connectivity:** inspect ordinary interface/route preservation, Ports
  rules, hook actions and desired roles. A diagnostic failure does not authorize
  an automatic backend switch or change network routing.
- **Package/profile inventory:** inspect `app_scan_diagnostics` and per-user exits.
  Incomplete inventory is not an empty installed-app set.
- **Ports:** distinguish IPv4/IPv6 apply receipts, actual `ports_state` and desired
  observers/policy. One family's success cannot certify the other.
- **Java attachment:** inspect current-boot LSPosed telemetry, hook-source identity,
  fixture attribution and loaded-process coverage. A worker PASS cannot update
  an already-running target process.

## 10. Privacy and content options

`forensics=false` omits raw sections and expensive forensic payloads. `appList`
only affects forensic `pm_packages`/`pm_users`; defaults are off for lean agent
reads, while explicit debug-export selection can include them. Redaction removes
those two sections, not every identifying value: canonical selections, logs,
Java artifacts and network addresses can still identify the environment. Inspect
selected attachments before sharing. Core credentials are not app-selected probe
arguments; no generic root path/command is accepted.

## 11. Kernel image bundle

Kernel images are separate explicit attachments. Inspect the acquisition manifest,
partition identity, byte/hash results and compression/packaging failures. An image
export is not boot-patch authorization, AVB/slot verification or permission to
write/reboot. No update or notification automatically flashes an image.

## 12. Code map

| Owner | Source |
| --- | --- |
| JSON model, schema and redaction | [VpnHideState.kt](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/debug/VpnHideState.kt), serialization golden test |
| Accepted diagnostic publication/export | [AutonomousDiagnosticInfo.kt](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/AutonomousDiagnosticInfo.kt), Core autonomous coordinator and [diagnostic-model](../crates/diagnostic-model/src/lib.rs) |
| Privileged observations and safe file handoff | [device_probes](../crates/core-daemon/src/device_probes), [diagnostics.rs](../crates/core-daemon/src/diagnostics.rs), [export.rs](../crates/core-daemon/src/export.rs) |
| Current capture/ZIP adapters | [DebugExport.kt](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/debug/DebugExport.kt), Java/kernel attachment adapters |
| Core recording lifecycle | [logcat_session.rs](../crates/core-daemon/src/logcat_session.rs) |
| Unprivileged native observations | [checks](../crates/checks/src/lib.rs); collectors return typed observations, not Pass/Fail verdicts |

<a id="automatic-attachment-planning-rpc-13"></a>

## Automatic attachment planning

[Shared Rust attachment policy](../crates/core-rpc/src/attachment_policy.rs) uses
bounded typed Core boot, implementation and accepted assessment inputs. The local
FFI adapter supplies one projection to APK consumers; Kotlin keeps source joins,
localization and the open sheet's explicit overrides. Unjoined/stale leaks cannot
recommend attachments, and recommendations neither collect them nor authorize
backend/boot changes. Capture/export/agent orchestration remains incomplete;
current APK packaging is not presented as a completed Core-owned session.

## Capture clock limits

Paired RPC31 adds the typed logcat auto-stop reason `clock_unavailable`. Core uses
CLOCK_BOOTTIME for temporary logging authority and the recording cap, so suspend
does not extend either. Active waits recheck at most every250ms after resume;
no cleanup is claimed while the CPU sleeps. With clock failure, duration is the
last valid measured lower bound. Field names stay stable; schema11 records the
changed elapsed-time meaning. Private journal14 fences older writers from the new
persisted recorder reason; schemas1–13 still migrate without discarding holds.
