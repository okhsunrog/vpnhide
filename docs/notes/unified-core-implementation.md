# Unified Core implementation

Approved design, based on dev-2.x `3493354f` (2026-11-02).
This is an implementation ledger, not a claim of device acceptance.

## Invariants

Core owns capability assessment, selection, activation and validation. Bundled
payload availability is not installed/live ownership and is never a conflict by
itself. Preserve CommandEngine's single writer, durable admission and immutable
receipts, isolated 0.x/1.x imports, and the APK's no-su boundary.

Initial policy: compatible built-in, eligible exact-KMI ko, authenticated compatible
KPM, then eligible Zygisk. Eligibility checks the entire desired policy, capacities,
required coverage, architecture and actual runtime evidence. Unknown effects block
fallback. No automatic kernel provisioning simply because KPM is absent.

Logical disable differs from unload. Known empty kernel policy prevents new
filtering decisions but does not drain in-flight work. ko/KPM coexistence is not
made safe by empty targets. Zygisk policy replacement does not revoke existing
process hooks. Transitions must expose process/reboot requirements truthfully.

Root observations do not prove Android app hiding. No APK, no VPN, stale measurements,
Java/Ports failures and unowned coverage gaps do not blacklist Native. Delayed health
changes create new operations, not retroactive changes to confirmed receipts.

## Ordered stages

| Stage | Status | Acceptance |
| --- | --- | --- |
| 1. Payload catalog and pure selection | Implemented as internal API; production integration follows stages 2–4 | Strict bounded manifest, pinned file verification, deterministic eligible candidates, unknown/owner guards, policy/capacity tests |
| 2. Durable backend lifecycle | Implemented for the current selector; bundle integration follows stages 3/4 | Initial effects journaled before dispatch, crash reconciliation, bounded attempts surviving worker restart, no blind replay |
| 3. Unified ZIP and CI assembly | Planned | All seven KMIs, both Zygisk ABIs, manifest/hashes, verified provenance, active/staged separation |
| 4. Zygisk boot authorization and fallback | Planned | Stale boot authorization denied, existing process effects accounted for, only safe transitions |
| 5. Typed state/RPC/UI/diagnostics | Planned | Paired contract bump, reasons/attempts/coverage, no misleading component health |
| 6. Shared maintenance plan/executor | Planned | Migration/repair/update use the same typed steps, preserve sources, manager-native staging/removal, durable progress/reconciliation |
| 7. Separate runtime provisioning | Planned | Verified fork/tool tuple, actual KALLSYMS=y, validated ALL=n support, durable original backup, plan-bound consent, correct write/readback errors |

Stages 1 and 2 establish internal primitives before the unified bundle becomes the
production selector. The existing separate-package path must remain truthful until
assembly and lifecycle wiring are ready; do not silently reinterpret RPC13 fields.

## Packaging and maintenance

One manager identity `vpnhide_core`, passive ko/KPM payloads and manager-discoverable
Zygisk libraries whose hooks require current-boot Core authorization. Do not bundle
KPatch-Next itself. Unchanged compatible payloads need not acquire every app release
number; preserve existing kernel control compatibility.

Keep installed legacy inventory for repair, separate from payload selection. Repair
only positively identified obsolete VPN Hide modules, preserve old config, prepare
verified replacement before destructive cleanup, and account for loaded hooks until
reboot. A shared maintenance plan serves 'Repair configuration' and 'Update Core'.
Start with one-button update and shared progress; automatic update requires prior
bounded opt-in. Boot patch/flash always has separate explicit consent.

An absent/incompatible Core cannot bootstrap itself through unavailable RPC. Initial
manager install remains explicit. Stage updates without mixing active tool generations;
keep journals/backups outside module directories. Restoring files is not transactional
rollback of resident hooks or a boot image. Real boot recovery may require external tools.

## Validation and limits

Host fixtures: corrupt/missing/mixed payloads, symlink/path races, capacity/default-mask
rejection, external owners, auth/unknown outcomes, crashes before/after every effect,
stale Zygisk policy/processes, profile UID changes, interrupted updates and RPC skew.
QEMU validates kernel vectors, not Android manager lifecycle or Zygisk injection.
Device gates: manager install/update/remove, SELinux, 32-bit apps, profile routing,
reboot, old built-in and direct 0.x/1.x upgrades. Provisioning additionally needs
supported device/slot/OTA/root-patch cases and independent recovery.

No device operations are authorized by this implementation plan. Creating a separate
public fork needs a concrete publication target agreed with the maintainer. No main
merge, release, force push or credential changes are part of these stages. Prior
Actions API access was forbidden; local checks do not establish CI success.

## Stage 1 evidence

`core-daemon/src/payloads/` implements bounded strict manifests, descriptor-relative
no-follow verification, retained payload descriptors and a pure deterministic selector.
Eleven regression tests pass. Workspace Clippy (all targets), fmt and diff checks pass.
Workspace tests: 456 passed, two existing Zygisk self-read tests failed in this
environment (`process_vm_readv` returns ENOSYS). No tests were weakened or skipped.
The production selector, RPC13 and backend wire remain unchanged in this stage.
No Android/device validation or observed GitHub CI result is claimed.

## Stage 2 initial checkpoint (historical: 54cb4940)

Initial activation now runs on CommandEngine, with synced intent before startup
side effects and synced completion in the existing private journal (schema 3,
reads schema 1/2). Interrupted startup survives worker restart, reboot and receipt
retention as a Native resource hold; storage failure prevents new dispatch.
Completed same-boot decisions come from the journal and recheck runtime identity,
not the best-effort native_boot.json file. Existing Persist/Native/Secret holds
also gate initial startup. RPC13 and backend control-v2 remain unchanged.

At that checkpoint, remaining stage-2 work was exact prepared payload/transition intent,
per-candidate attempt evidence and safe recovery of interrupted startup. The initial
checkpoint deliberately refuses replay, including across reboot; it does not yet
provide an automatic way to resolve every startup hold. Do not delete the journal
as a recovery mechanism. Later lifecycle/recovery work must reconcile or supersede
this evidence through a checked operation. Unified selection and automatic fallback
are not enabled by this checkpoint. Downgrading to schema-2-only core is refused.

Stage 2 initial-startup checkpoint validation: nine new startup/reconciliation tests
pass, including schema-2 upgrade, retention, live-identity mismatch and KPM auth-loss
recovery. Final workspace run: 465 passed, two unchanged Zygisk self-read failures
(ENOSYS environment limitation). Workspace Clippy/all-targets, fmt, diff and module
scaffold/no-su checks pass. Core ZIP builds for arm64 and armv7. No Kotlin/C changes;
JVM and device checks were not rerun. No observed CI outcome is claimed.

## Stage 2 continuation: prepared transitions and recovery

Implemented: schema 4 records exact prepared payload
SHA-256/size/source and transition, boot/config generation, load-time feature,
and pinned Zygisk retirement directory. Anonymous private payload snapshots keep
loader bytes independent of subsequent module-path replacement. Load and credential
recovery both pass through CommandEngine's durable Native phase boundary.

Bounded history retains 16 superseded attempts; independent per-boot load budget
and payload quarantine prevent history pruning or restart from permitting replay.
A failed load can have effects and remains held. Existing explicit RepairActivation
can adopt a compatible live backend without a second load, or load after reboot;
reboot alone does not retry. Unknown historical receipts/attempts stay historical.
Missing settings can still be explicitly imported while only startup is held.
Legacy source cleanup and best-effort-file restoration were removed from startup.

Schema 1/2/3 journals upgrade without discarding holds; unsupported/corrupt journals
remain untouched and block mutations. Schema-2/3-only Core cannot downgrade-read
schema 4. Restore a compatible Core, not an empty journal. RPC13, canonical schema1
and backend control-v2 are unchanged. Kernel binary identity is not observable via
control-v2: live adoption establishes compatible ownership, not an exact binary hash.
Host FD-loader tests do not validate Android SELinux access to the retained FD.
Unified bundle selection and automatic fallback remain stages 3/4, not implemented
by these lifecycle primitives.

Stage 2 final host validation: 471 Rust tests passed; the two unchanged Zygisk
self-read tests fail because this environment returns ENOSYS for process_vm_readv.
Regressions cover exact payload bytes after source replacement, symlink rejection,
three real process crash boundaries, same-boot live adoption without reload,
reboot plus explicit recovery with a changed payload, absent-config bootstrap,
history pruning versus independent load budgets, schema-3 hold preservation,
corrupt/future schema refusal, and KPM appearing between selection and dispatch.
Workspace Clippy/all-targets, fmt, diff, no-su and module scaffold checks passed.
No APK/Kotlin contract changes; JVM/device tests were not run for this stage.
No GitHub CI status is observable through the previously denied Actions API.

Core ZIP cross-build passed for arm64 and armv7 against the final stage-2 runtime.
