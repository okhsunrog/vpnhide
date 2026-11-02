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
| 3. Unified ZIP and CI assembly | Implemented passive assembly; real seven-KMI CI/device acceptance unobserved | All seven KMIs, both Zygisk ABIs, manifest/hashes, verified provenance, active/staged separation |
| 4. Zygisk boot authorization and fallback | In progress: durable authorization/revocation tested; production selector/fallback unfinished | Stale boot authorization denied, existing process effects accounted for, only safe transitions |
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

## Stage 3: unified assembly (runtime integration remains stage 4)

Each producer stamps `build-provenance.json` with its actual checkout commit,
tracked dirty state and exact staged-file hashes. `scripts/payload_bundle.py`
requires clean same-commit artifacts for seven KMIs, KPM, both Core ABIs and both
Zygisk ABIs. It validates hashes, size budgets, module identity and ELF machine;
rejects duplicate/unsafe/symlink members; streams explicit members into a private
staging tree and replaces the output only after success. This is trusted-build
provenance, not a publisher signature or a kernel ABI compatibility guarantee.

The unified ZIP has only `id=vpnhide_core`. Its `payloads/manifest.json` is consumed
by the stage-1 catalog. Zygisk remains under `payloads/zygisk`, outside manager
autoload: no injection is enabled before stage-4 authorization. Backend scripts,
module identities and update URLs are not copied into the Core bundle. Existing
legacy installed-backend runtime selection still runs until stage 4 is wired;
this packaging step does not claim automatic activation or fallback. Active versus
manager-staged generation must be resolved by lifecycle integration, not by reading
`modules_update` payloads as active.

CI builds all bundle dependencies when any bundled component changes. The Core
assembly job consumes only this run's artifacts and validates the exact checkout
SHA; its output remains `vpnhide-core.zip`. `vpnhide-core-base` is an intermediate,
not the complete distribution, and is excluded from release assets. The existing
signed-APK artifact gains a commit/run/hash sidecar; signing rules are unchanged.
No separate kernel runtime or KPatch-Next is bundled. Existing separately packaged
backend artifacts remain available during the transition and are not declared active
merely because their payload bytes exist.

Local command after producing clean component ZIPs from the same checkout:

```sh
python3 scripts/payload_bundle.py --inputs dist-inputs --output dist/vpnhide-core.zip --commit "$(git rev-parse HEAD)"
```

Inputs: `vpnhide-core-base.zip` (rename core/build.py's daemon-only output),
`vpnhide-kmod-<kmi>.zip` for all seven supported KMIs, `vpnhide-kpm.zip`, and
`vpnhide-zygisk.zip`. Dirty/mixed-source inputs are intentionally refused.
Eight Python assembly tests and twelve Rust catalog/selection tests (including
Python-produced ZIP→Rust reader interoperability) pass. Actionlint structure and
repository Ruff checks pass. Full actionlint with shellcheck also reports pre-existing
SC2129/SC2012 warnings in untouched setup/QEMU scripts; baseline comparison confirms
they are not introduced here. The KMI argument quoting warning was fixed. Full real seven-KMI assembly is not locally verified: this
workspace has no DDK images/payload builds. CI results remain unobserved because
Actions API access was previously denied. Device/manager acceptance is not claimed.

Final stage-3 local gates: 472 Rust tests passed; the same two Zygisk self-read
ENOSYS failures remain. Clippy/all-targets, fmt, scaffold/no-su and diff checks pass.
Core and Zygisk cross-builds passed for arm64/armv7; KPM cross-build passed after
initializing its pinned KernelPatch submodule. Their metadata correctly marks
these working-tree validation artifacts dirty, so they cannot accidentally enter
the clean CI bundle. No kernel source or submodule revisions were changed.
Signing failure-path tests: 5 passed; no signing credentials were accessed.

## Stage 4 checkpoint: authorization boundary (not complete)

`protocol::zygisk_activation` defines a bounded private envelope containing a
kernel boot UUID, nonzero durable generation and exact control-v2 wire. This is
not a kernel protocol change. Decode rejects stale boot, malformed/future version,
noncanonical generation and invalid/oversized policy. Bundled policy rejects a
nonzero default or unsupported hook bits rather than masking them into eligibility.
Exact generation+wire comparison is available for journal readback.

The Zygisk reader explicitly distinguishes `vpnhide_core` with activation contract
1 from the separate legacy `vpnhide_zygisk` control-v2 recipient. Bundled mode never
falls back to legacy `targets.txt`; reads are bounded, descriptor-relative,
no-follow and regular-file-only. Core's publisher similarly refuses a raw wire
for the bundled recipient, and refuses a bundled envelope for the legacy recipient.
The envelope is atomically published as `zygisk-activation.txt` through the pinned
recipient directory. A single authoritative file avoids mixing header and policy
from separate writes. Authorization is root-owned state, not a cryptographic claim.

This checkpoint does **not** enable bundled autoload, publish authorization from
CommandEngine, wire production selection or implement fallback. Before those are
enabled, the writer must journal the exact generation/boot/wire tuple, reconcile
that tuple after crash, and durably revoke/supersede authorization before any
conflicting effects. The consumer treats the current file as authority; generation
validation against durable expected state belongs to the writer/reconciler. It
cannot independently detect a root actor replaying an older same-boot file.
Processes that already consumed authorization remain outside revocation guarantees:
no claim of hot-unload, atomic handoff or draining in-flight hooks is made.

Next stage-4 work remains production capability adapters + selector integration,
journaled authorization/revocation and process-aware transition/fallback gating.
Keep passive packaging until these gates pass. Stages 5–7 are still outstanding.

Authorization checkpoint validation: seven new regression tests; workspace 479
passed, with the same two pre-existing Zygisk process_vm_readv ENOSYS failures.
Final targeted policy tests, workspace Clippy/all-targets, fmt, diff and module
scaffold/no-su pass. Core and Zygisk cross-builds pass for arm64 and armv7.
No RPC13/kernel control-v2 change, no APK/device test or observed CI claim.

Production selector integration must derive each candidate's policy from its
existing kernel/Zygisk projection: their hook IDs are different. Do not apply one
kernel hook mask to all candidates or equate different interception coverage.
Full target/default-mask/capacity checks remain mandatory for each candidate.

### Stage 4 continuation: durable authorization and revocation

The command writer now derives authorization from the admitted operation ID and
current boot, pins recipient identity in NativeIntent, and syncs that intent before
publishing. Delivery refuses a changed directory. Reconciliation confirms only
same-boot, same-directory, exact generation+wire publication. Old/new generation,
missing file, incompatible recipient or reboot leave Unknown; they do not prove
that no process consumed an authorization. A legacy raw-wire intent cannot confirm
bundled publication. Explicit repair publishes a fresh generation and preserves
the historical Unknown receipt.

Journal schema 5 preserves earlier journals and validates that authorization
belongs to its operation. Schema-4-only Core refuses this journal; keep evidence
and restore a compatible Core rather than deleting the journal for downgrade.
The shared retirement path now removes bundled authorization through the verified
pinned directory and syncs it; legacy recipients retain empty-wire publication.
This revokes future authorization only. It does not establish process quiescence,
and cannot by itself authorize starting a competing backend.

The pure selector now accepts full per-candidate projections, preserving distinct
kernel/libc hook ID spaces. Missing/duplicate projections fail closed, as do
unsupported defaults and capacities. Read-only capability adapters and real family
projection adapters now exist, but production selection,
transition scheduling and safe fallback remain **unfinished**; bundled autoload
is still disabled. No fresh/stale diagnostic oracle has been added here.

Tests include real-process crashes before/after Native publication, old-generation
readback, subsequent repair, directory/boot/wire mismatch, operation-generation
validation, revocation while an old reader retains its file and per-candidate
projection gaps. Host fixtures do not establish Android injection or hot-unload.

The independent `zygisk_exposure_boot` journal marker outlives receipt/attempt
pruning. Same-boot kernel load/adoption is refused after possible Zygisk exposure,
including explicit repair. Revocation does not clear this marker; a new kernel
boot identity is the reset boundary. Legacy retained Zygisk startup evidence is
imported into the marker while upgrading the journal.

Capability evidence is conservative: exact known Android KMI, required module/
kprobe config and module-loading sysctl, actual authenticated KPM facts, and a
named `libzygisk.so` mapping in a root-owned zygote. The last probe is deliberately
narrow: hidden/anonymous mappings and unrecognized providers are Unknown, not
proof of absence. This is not a universal Zygisk adapter or a health measurement.
No manager-name heuristic, implicit runtime installation or load is added.
Family projections use one caller-supplied package/profile snapshot. The optional
filesystem bit carried by ko/KPM wire is not required coverage when its load-time
gate is explicitly off; the wire and every other requested bit remain intact.

Remaining stage-4 gates: connect verified active catalog and prepared selector to
startup/recovery; obtain useful loader-generation evidence across supported Zygisk
providers; journal safe transition ordering and bounded fallback; prevent stale
process telemetry from confirming new generations. Autoload remains disabled until
these contracts are completed. Stages 5–7 have not been implemented by this checkpoint.

Checkpoint validation: 490 workspace Rust tests pass; the two existing Zygisk
self-read tests fail in this container (`process_vm_readv(self)` returns ENOSYS,
confirmed directly). No tests were ignored or weakened. Workspace Clippy with
all targets and warnings denied, fmt, diff and module scaffold/no-su checks pass.
Core cross-builds pass for ARM64 and ARMv7 (validation ZIP only; dirty-build
provenance is intentionally not accepted by the release bundle assembler).
No Android process-injection, reboot/fallback, QEMU/KMI or device acceptance was
run here; GitHub Actions results remain unobserved. RPC13 and kernel control-v2
are unchanged; the private journal upgrade to schema 5 requires compatible Core.

### Stage 4 continuation: frozen execution and PM readiness

`Prepared` now owns the exact `Plan`; production execution and recovery preflight
no longer call the selector a second time from diagnostic facts. The test-only
legacy wrapper still exercises the old decision matrix. A regression test changes
facts after preparation and proves they cannot replace the prepared decision.

The production boot-apply path publishes an explicit PackageManager wait before
selection/loading and performs the wait on the boot thread, outside the command
writer. RPC remains available; waiting does not create a startup attempt or spend
the load budget. On readiness, startup enters the same journaled writer path.
A real-process host regression exercises PM failure followed by readiness, reads
RPC during the wait and verifies the durable selection afterwards. Host fixtures
without boot apply retain explicit immediate startup. The boot-apply CLI follows
the same PM-before-selection ordering.

Zygisk's cached snapshot retains only the consumed authorization generation (not a
second full wire copy). Its app heartbeat emits `authorization_generation`; legacy
policy leaves it empty. Replacement/revocation cannot relabel an old process as a
new generation. This field is evidence for the future typed observer, not a new
claim of healthy hooks or an enabled automatic fallback.

Provider contract review: Magisk's public Zygisk API provides module callbacks and
an ABI-aware root companion, callable during pre-specialization. ZygiskNext promises
Zygisk API compatibility, not Magisk internals; NeoZygisk documents the same API
compatibility. Therefore the next loader proof must use our own module/companion
handshake rather than infer readiness from manager files or mapping names:
- <https://github.com/topjohnwu/zygisk-module-sample/blob/master/module/jni/zygisk.hpp>
- <https://github.com/Dr-TSNG/ZygiskNext/blob/master/README.md>
- <https://github.com/JingMatrix/NeoZygisk/blob/master/README.md>
No companion handshake or provider device acceptance has been implemented here.
Validation: 493 workspace Rust tests pass, with only the two known Zygisk
self-read ENOSYS failures in this container. Workspace Clippy/all-targets with
warnings denied, fmt, scaffold/no-su and Core/Zygisk ARM64 + ARMv7 builds pass.
These are host/cross-build results, not observed CI or Android acceptance.
Production catalog integration, transition/fallback scheduler and stages 5–7 remain
unfinished. Bundled autoload remains disabled. RPC13/control-v2/journal5 are unchanged.

## RFC source documents (status at import)

- [RFC 0001: automatic GKI/KMI selection](../rfcs/0001-automatic-gki-kmi-selection.md)
  retains its original **Proposed** document status. Implementation of its goals was subsequently authorized below; that does not establish prototype correctness. Its
  kernel-loader preflight requires separate review and kernel/device validation;
  importing the RFC does not enable it. Unknown or ambiguous KMI must not resolve
  to an arbitrary first candidate.
- [RFC 0002: autonomous diagnostics](../rfcs/0002-autonomous-diagnostics.md) is
  **Draft**. Native TUN and framework fixtures, Android background diagnostics and
  orchestration/cache are proposals; framework feasibility still needs a prototype.
  Internal decision labels are not evidence of working code or authorization to
  add these experiments to the current implementation.

## Current-cycle scope update (2026-11-02)

The maintainer authorized implementing both RFC goals in this architecture cycle,
before the first beta, with iterative phone testing once the device environment is
actually authorized. This is implementation authority, not proof of feasibility,
passed acceptance or permission for live boot flashing. The imported RFC texts remain
historical proposals; track corrections and actual evidence here rather than rewrite
the author's source. No phone/laptop tests have been performed in this cloud checkout.

Continue the same vertical Core selector/lifecycle first, then integrate these work
items into stages 4–7 and typed diagnostics; do not create competing selectors:

1. Finish verified catalog → complete per-family policy → Prepared → durable execution,
   own Zygisk loader/process evidence and safe bounded fallback.
2. RFC 0001: manifest-derived KMI candidates and explicit evidence/ambiguity. Prototype
   final-artifact preflight only after a hash-bound protocol-capability check; unknown
   kernel module parameters can be ignored and run a normal init. Verify earliest
   return, no hooks/proc state and cleanup across representative QEMU kernels before
   enabling any production experiment. Evaluate whether import/CRC evidence actually
   distinguishes candidates; never select the first of multiple passing candidates.
3. RFC 0002: minimal end-to-end native TUN and framework-fixture prototypes, then
   coverage expansion. Expected checks derive from the requested contract, not the
   hooks that happened to install. RAW control must be isolated without disabling
   protection for an entire UID; non-VPN preservation control is mandatory. A fresh
   diagnostics-worker PASS cannot prove that already-running games consumed new policy.
   Framework fixture feasibility remains an explicit hypothesis until demonstrated.
4. Join evidence through the existing Core owner, typed RPC/UI/export and maintenance
   flow, with cache invalidation by actual boot/config/process generations. Continue
   migration/repair/update and separate runtime provisioning in this same beta cycle.

Each prototype needs deterministic host failure/crash tests, relevant cross-builds,
QEMU where applicable and explicit Android acceptance evidence. Missing device access
blocks only those device checks, not independent code/prototype work. Experimental
kernel execution, installations, root changes, reboot and flashing on the actual phone
still require authorization for the concrete action; no automatic flash is implied.


## Stage 4 handoff: catalog-bound preparation (2026-11-02)

Production preparation recognizes an active complete Core catalog, projects all
candidate policies from one package inventory, and freezes the eligible candidate,
exact payload digest/bytes, manifest digest/source commit and wire policy in the
startup intent. An ambiguous KMI or insufficient policy coverage holds selection.
Restart accepts only the same verified active catalog; replacement in the same
boot requires explicit recovery, never implicit re-selection or load replay.
First configuration import and explicit Repair can recover an initial no-config
hold. Journal schema 6 reads older journals; older Core must not be made to run by
deleting the journal. Preserve state and use a compatible Core for recovery.

This is a checkpoint, not completed stage 4 or a ready universal installation.
Bundled Zygisk eligibility remains Unknown and its libraries remain passive until
an own-loader handshake, journaled authorization and typed generation observation
are connected. Actual post-load coverage validation and controlled fallback remain
to be completed before declaring selected == healthy. Unknown effects and prior
Zygisk exposure remain guarded. Stages 5–7 and both RFC implementations remain.

The next writer must also test initial import/recovery against a complete bundle,
finish generation-aware telemetry consumption, and validate built-in coverage on
the phone before removing an existing working backend configuration. Host fixture
payloads establish preparation and hash binding, not kernel loader compatibility.


Handoff checks: `cargo test --workspace --no-fail-fast` completed with 497 passing
and the same two Zygisk self-read failures caused by this host's
`process_vm_readv` ENOSYS limitation. No tests were weakened or skipped. Workspace
Clippy (`--all-targets -- -D warnings`), formatting, module scaffold and APK no-root
execution checks passed. APK/Kotlin and kernel payload sources are unchanged in
this checkpoint; device acceptance and CI status are unverified.

Core cross-build passed for ARM64 and ARMv7. The resulting local ZIP is a dirty
validation build, not a clean same-commit full bundle for distribution. Full bundle
assembly still requires all seven KMI outputs, KPM and both Zygisk ABI artifacts
from the same clean commit. No actual full-matrix build or device run is claimed.
