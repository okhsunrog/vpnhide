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
| 2. Durable backend lifecycle | Planned | Initial effects journaled before dispatch, crash reconciliation, bounded attempts surviving worker restart, no blind replay |
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
