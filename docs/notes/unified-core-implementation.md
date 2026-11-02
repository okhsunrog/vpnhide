# Unified Core implementation

Approved design, based on dev-2.x `3493354f` (2026-11-02).
This is an implementation ledger, not a claim of device acceptance.

Current cloud contract: **RPC22 / journal11 (reads schemas1–10)**. Kernel control2 /
telemetry1 is unchanged: APK/Core contract and private journal upgrades do not
require rebuilding a compatible user builtin kernel. Stages4–7/RFC2 remain partial;
the latest entries distinguish remaining code from device/key/publication gates.

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
| 4. Zygisk boot authorization and fallback | In progress: authenticated loader, durable authority and exact process consumption; shared revocation/fallback executor pending | Stale boot authorization denied, existing process effects accounted for, only safe transitions |
| 5. Typed state/RPC/UI/diagnostics | In progress: RPC22 native/autonomous projections shared by Dashboard/picker/Statistics/agent/export; expanded RFC2 health evidence remains | Paired contract bump, reasons/attempts/coverage, no misleading component health |
| 6. Shared maintenance plan/executor | In progress: source-bound legacy migration/archive; replacement/manager/reboot vertical pending | Migration/repair/update use the same typed steps, preserve sources, manager-native staging/removal, durable progress/reconciliation |
| 7. Separate runtime provisioning | In progress: offline evidence/write model and pinned fork patches; concrete adapters pending | Verified fork/tool tuple, actual KALLSYMS=y, validated ALL=n support, durable original backup, plan-bound consent, correct write/readback errors |

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


## Local main and first Android acceptance (2026-11-02)

Development is consolidated in `/home/okhsunrog/code/vpnhide`, branch `main`.
Commit `045b58a439d62c2a1dbd407321d60a51ce8cb87b` preserves both pre-rebase
and final development histories; its tree equals cloud checkpoint `161dda99`.
The redundant `vpnhide2` worktree and local/remote `dev-2.x` branches were removed.
User IDE files were retained. Future work stays in main.

Pixel 8 Pro: Android 17/API 37, android14-6.1 kernel, KernelSU Next. Same-commit
signed APK and complete CI Core bundle were installed after checking all ten
payload hashes/provenance and matching the installed APK certificate. Legacy
`vpnhide_ports` and `vpnhide_builtin` were removed through ksud; KPatch-Next and
other modules were untouched. A user-authorized reboot completed and user 0 was
manually unlocked. Local APK build and 889 JVM tests passed before installation.

Read-only acceptance confirmed RPC 13 handshake, control-v2 projection, journal
schema 6, completed builtin adoption, matching installed daemon hash and catalog
source commit, all ordinary kernel hooks, and Ports IPv4/IPv6 delivery. Both actual
firewall chains contain the OUTPUT jump and 36 UID selectors. Current projected
Native wire exactly matches the journaled startup wire (47 unique UIDs). Canonical
schema-1 config (58 package records) remained byte-identical to its pre-install
backup. No reset or direct privileged config edits were used.

The first logcat was cleared under the existing recording instructions before the
user's preserve-logs follow-up arrived. The original first-opening APK messages
cannot be recovered from that buffer. A subsequent cold start reproduced the
same startup error; Core logs/journal/config and later Retry logs were preserved.
At 06:55:09 startup preparation competed with the initial config-publication root
refresh; Core's reserved slot rejected one read. At 07:02:42 Retry completed the
first diagnostic suite, then the unconditional after-paint RepairActivation caused
Native/Ports publications, two more root reads and another suite at 07:02:44.
The daemon epoch stayed unchanged; this evidence does not indicate a daemon
restart or broken builtin hooks. Remove the competing initial refresh and the
unconditional startup repair; do not hide invalidations with timers or debounce.
The fix still needs a separately agreed APK replacement and device re-check.

CI access for this local cycle used host-side authenticated `gh run list`,
`gh run view` and `gh run download` on nova under approved host execution. It did
not use the cloud worker's Actions API channel. Subsequent notice of an earlier
cloud Actions refusal is preserved; no further Actions access is assumed here.

## Additional authorized follow-up

After triaging the startup/flicker defect, pin all project Rust components and
build/CI tooling to Rust 1.99.1 in a separate commit, verify official availability,
and fix new Clippy findings semantically. Run fmt, all-targets Clippy, tests and
relevant Android ARM builds without weakening checks. Do not install another
system-component build or reboot the phone until the next concrete cycle is agreed.

Continue stages 4–7 and both RFCs in this pre-beta architecture cycle. After
autonomous diagnostics, move shared export/report orchestration, agent device
operations/read models, then remaining configuration transforms into Rust. Android
UI, platform integration and app-context probes remain Kotlin. Root logcat ownership
already belongs to Core. This transfer is accepted scope, not completed work; do
not start the broad move while the current startup regression is under diagnosis.


The Kotlin-to-Core transfer also moves domain tests with their owner. Evaluate
and remove redundant or content-free tests only where equivalent useful coverage
is established; retain Kotlin/Core contracts, Android-specific tests, regression
cases and migration coverage. Test counts and green runs are not deletion goals.
The separately requested read-only Kotlin audit supplies the concrete transfer
matrix; it does not change this checkout or block independent toolchain work.


## Kotlin audit integration and updated migration order

The complete [Kotlin/Core ownership audit](kotlin-core-architecture-audit.md) is
preserved verbatim from Library version 0 (56,822 bytes), pinned to source
`045b58a439d62c2a1dbd407321d60a51ce8cb87b`. Its measurements and line references
remain historical. Startup fix `32a35328` already removes the initial competing
inventory refresh and once-per-APK automatic RepairActivation; it does not yet
replace DiagnosticsCache.run with autonomous diagnostics. The remaining audit
items below are planned work, not implementation or device acceptance claims.

Continue the authorized pre-beta cycle as complete vertical slices:

| Order / owner | Work and removal boundary | Required evidence |
| --- | --- | --- |
| Stage 4 + RFC1 | Extend the existing verified catalog/Prepared/journal selector with own Zygisk loader handshake, process generation observer and bounded fallback; then hash-capability-bound KMI preflight. Delete the APK manual ZIP/KMI chooser. | No first-passing ambiguity resolution; separate preflight/load budgets; no leftover hooks/proc state; QEMU and explicitly authorized device experiments. |
| Stage 5 typed health | Publish BackendState, PolicyHealth, DiagnosticState and one locale-independent ProtectionAssessment through existing generated RPC. Move native/feature/module/LSPosed classification and statistics attribution to that owner. | UI, Agent, settings and report agree for one epoch/revision; stale/unknown/partial evidence never becomes healthy. SQLite enabled/scope observation belongs to a bounded Core adapter with consistent DB/WAL/SHM handling. |
| Stage 5 + RFC2 | Prove Java callback/framework fixtures and real Zygote worker launch alongside bounded native fixtures; implement the Core run manager, report and fingerprint/cache, then remove local diagnostic ownership and external-VPN admission. | RAW positive stimulus, protected differential, non-VPN preservation, complete required plan and cleanup; loaded code, process/system_server and Connectivity APEX generations. Fresh worker PASS does not prove existing target processes consumed policy. |
| After autonomous diagnostics: common export/report | Move capture/session planning, baseline-before-run, accepted report joining, attachment recommendation and delivery lifecycle from APK orchestration to a shared Rust owner. NetworkViewData.evaluateNetworkView and ExportRecommendationData are pure policy. | One capture ID and report; crash/lease restoration, bounded artifacts, retained delivery/ACK and descriptor-relative file safety. Root logcat ownership already exists and must not be rebuilt. |
| Next: Agent operations/read models | UI and Agent share typed intentions, validation and the same report/statistics projections. Keep opt-in HTTP gateway unprivileged. | Contract tests for argument semantics, authorization, unknown enums, reply loss and consistent publication. Never add a privileged arbitrary path/command endpoint. |
| Next: remaining transforms / stage 6 entry | Replace CanonicalEditData.applyCanonicalEdit with the existing Rust reducer; migrate auto-hide/provenance, self-target preparation and complete inventory accounting. Preserve local drafts, pending UI and prevention. | Shared fixtures for partial profiles, shared appId, reserved self and per-family projection. Remove duplicate reducers/parsers only after consumer switch and parity. |
| Stage 6 maintenance | Plan-bound direct 0.x/1.x import, bootstrap, repair and update using the existing single writer and durable recovery. | Sources survive failed replacement; interruption/reply loss does not replay uncertain effects; journal is never deleted to unblock recovery. Initial absent Core retains a manager/bootstrap adapter. |
| Stage 7 provisioning | Separate verified runtime/kernel provisioning plans and concrete consent for boot effects. | Fork/tool tuple, KALLSYMS/ALL, backups, slot/OTA/recovery and loaded-code evidence. Missing KPM alone never authorizes flashing. |
| Hook-local rules within these slices | Share pure Java policy/callback/cover decisions in a local Rust library or derived immutable policy. | Android attachment, Binder objects, callback registrations and telemetry remain Android adapters. No root RPC on every Binder callback. |

### Capacity: immediate UI prevention is retained

The maintainer explicitly requires preventing an over-capacity selection while the
user is picking applications, rather than discovering it only at Save. Kotlin
keeps that immediate draft/selection behavior. Core supplies typed limits,
capabilities and enough generation-bound accounting (unique UIDs, profiles,
shared-appId contributions, reserved self and wire/profile constraints), or a
shared local evaluator/draft preview, so no second hardcoded 160/1 authority
remains. A package count is not a native-slot count. No mandatory RPC per tap.
Core still validates the complete projected candidate at commit and for Agent
mutations, including state changes after a preview. An already-over-capacity draft
must allow reductions. The audit's request to remove the Kotlin capacity mirror
does not remove local UI prevention.

### Test migration and deletion ledger

Move domain behavior tests with each new authoritative owner; retain Kotlin/Core
contracts, Android boundary tests, draft/ACK/reconnect fencing and regressions.
Each removed test records its remaining duplicate, stronger executable
replacement, new owner/parity vector, or deliberately changed RFC acceptance case.
Do not delete a failure to obtain a green run or target a percentage reduction.

Concrete audit candidates to assess in their own commits:

- Consolidate the repeated four-case verdict table in NetworkViewCheckTest and
  the repeated version equality in DetectModuleMismatchesTest; preserve callback
  history/handle isolation and null/presence cases.
- Replace HookCheckCoverageTest's empty-set tautology with actual registry
  validation when exemptions exist; retain every-kernel-hook coverage.
- Replace DebugShellSnapshotTest/RootSnapshotCacheTest source.contains checks
  with executable Rust collector/output and command-spy contracts, preserving
  honest identity and the no-expensive-inventory counter path.
- Keep golden/discriminator, atomic telemetry, mutable FFI snapshot, lost-reply,
  migration, PartialHookGap and system_server package-boundary obligations.
- Replace old manual-KMI and clean-without-RAW expectations with RFC1 ambiguity
  and RFC2 positive-control/preservation/missing-required-hook acceptance cases.

No large Kotlin migration or test deletion is included in the startup/toolchain
fixes. The next device cycle can be APK-only: replace the signed APK over user 0,
cold-start/Retry under preserved logs, and verify one startup root read, no automatic
RepairActivation and no redundant confirmation loop. Running Core RPC13/control2
remains compatible; a Core replacement/reboot is not a prerequisite for that test.
Further device changes still await agreement on that concrete cycle.


## Pause for cloud continuation (2026-11-02)

The maintainer is taking the phone and laptop offline. Stop local/device work;
continue the large architecture task in a fresh cloud checkout of main. Startup
fix is commit `32a35328`; the separate Rust 1.99.1 upgrade is commit `8a0eb8d7`.
The toolchain commit includes workspace/Zygisk MSRV, rustup pin, CI image/cache/path
filters, reviewed Clippy rewrites, and NixOS-portable isolated test fixtures.

Completed locally on the final Rust sources: fmt, host all-targets Clippy with
warnings denied, all 499 workspace tests (zero failures/ignored), strict ARM64 and
ARMv7 Android Clippy, and x86_64 app/Core Android Clippy (Zygisk remains ARM-only).
Three tiny Android TLS scopes use checked expect for the independently reproduced
Rust 1.99.1 false positive against an already const thread_local initializer;
unfulfilled expectations remain errors. No global lint suppression or removed
behavior assertions. Startup APK validation separately passed 890 JVM tests,
detekt, CPD, ktlint and assembleDebug before the toolchain commit.

Not completed after the Rust upgrade: fresh release ARM packaging, a new signed
APK build, a full same-commit unified Core bundle, or device acceptance of the
startup fix. Do not present the earlier signed validation APK or daemon-only base
ZIP as those final artifacts. No additional phone install/reboot is authorized
at this pause. The phone still runs the tested `045b58a4` APK/Core; configuration
was unchanged, builtin and Ports confirmed. Follow-up can be APK-only after an
explicitly agreed cycle; do not require replacing compatible Core just because
version strings differ. Keep RPC/control/journal compatibility and exact artifact
identity checks.

Local signing credentials stay on the laptop and were not copied to cloud or the
repository. Native runtime evidence and logs are local task artifacts; the tracked
summary above contains the nonsecret findings needed for cloud implementation.
Do not retry the earlier denied cloud Actions API via another route. Local host gh
access in the first install cycle was recorded separately and is not cloud access
authority. Validate fresh cloud capability normally and report concrete blockers.


## Cloud continuation: build recovery and final startup gate

Current cloud source starts at `2e458a26bc3bdfd1cf40e4f80d9ca71f6d11e083`,
a descendant of the handoff commit with one subsequent Actions dependency update.
Fresh Rust 1.99.1 release Core (ARM64/ARMv7), both Zygisk ABI packages and KPM
built. Three baseline KMI packages built; the remaining DDK matrix currently hits
the 32-GiB executor's VFS image/container duplication limit. Preserve completed
clean provenance artifacts; this is not a complete bundle or observed CI result.
Production APK signing is unavailable: no signing key is configured in cloud.
No temporary production identity and no laptop/device access were used.

The official rustup toolchain makes the earlier Android TLS lint expectations
unfulfilled. Removed those expectations; strict ARM64/ARMv7 Clippy passes without
lint exemptions. The baseline workspace has 497 passing tests and the same two
Zygisk self-read failures (`process_vm_readv` is unavailable in this sandbox),
with zero ignored tests. This does not invalidate the separate local 499-pass run.

Latest UI decision: the maintainer considered optimistic per-toggle autosave and
then explicitly cancelled it. Keep picker Save/Cancel and local drafts. Immediate
capacity prevention remains; use Core-owned accounting/shared local projection.
No autosave code was applied. Do not revive that superseded proposal.

Final acceptance must specifically cover the reported blocking startup message
`core root snapshot probe exit=-1`: cold startup, delayed Core readiness, concurrent
root-read admission, reconnect and Retry. A matching message on the old phone APK
is not proof of one cause. Run regressions on the final sources; do not turn failed,
truncated, stale or unavailable data green. Device validation remains pending.


### Startup read ownership completed in cloud

The screenshot's exact `core root snapshot probe exit=-1` is a generic failed
probe result, not a unique diagnosis. Startup now joins the existing root
observation owner rather than directly competing for Core's one root-probe slot.
Invalidation drains the physical worker before its successor. Removed obsolete
whole-snapshot/inventory seeding and redundant post-write caller invalidation.
Retry creates one fresh generation; delayed Open does not repeat successful
preparation; reconnect refreshes identical facts after an offline read failure.
The process's restart requirement is sticky. Typed version failures, bounded
probe output and truncation survive the shared read boundary without false green.

Validation after review fixes: all 894 JVM tests passed, zero skipped/failures;
warnings-as-errors Kotlin compilation, ktlint, detekt, CPD and Android lint passed.
New executable tests cover startup/inventory joining and drain, package changes
between Retry reads, delayed Open, reconnect, concurrent Retry, protocol mismatch,
truncated output and retained failure causes. Replaced the historical startup
whole-snapshot seed test with actual shared-reader/refresh behavior. Phone still
runs the older APK; cold-start device acceptance is expressly pending. This gate
must be rerun again after later architecture changes, not considered final beta
acceptance merely because this checkpoint passed.


### Shared native policy and picker prevention

Native selection/mask projection, profile/shared-UID accounting and limits now
live in unprivileged `crates/native-policy`. Activator delegates to that owner;
the APK links only its generated local FFI preview, not privileged activator code.
Accepted Core inventory completeness and **selected** backend supply the preview;
observed live backend and local icon-scan UIDs are not substituted. Both UID count
and grouped wire bytes are checked. The local Rust subset predicate permits
reductions even if regrouping grows an already-overflowing wire, or inventory is
incomplete. Core additionally refuses oversized native wire before Persist.
RPC13/kernel control2/journal6 remain unchanged by this additive local FFI API.

Test migration: replaced eight Kotlin native-accounting tests with six direct
native-policy behavior tests, retained activator parsing/adapter coverage and Core
no-partial-write process tests, and added five Kotlin interaction/real-generated-FFI
contracts. Shared UID, profiles, platform UIDs, actual main-user self, self/shared
identity, uninstalled and missing-picker packages, empty family masks, filesystem
gates, byte overflow, selected-versus-observed backend and group-growth reductions
are covered. The old unconditional synthetic self-slot expectation is deliberately
replaced by actual self UID accounting. No source.contains guard was substituted.
Full cloud Rust run: 503 pass, two unchanged sandbox process_vm_readv failures,
zero ignored. Strict host and both ARM ABI Clippy pass. Kotlin final gate rerun
includes startup acceptance and remains separate from phone acceptance.

### Baseline build gates recovered

A full clean source-consistent baseline bundle was assembled for
`2e458a26bc3bdfd1cf40e4f80d9ca71f6d11e083`: all seven KMI variants, KPM,
both Zygisk ABIs and both Core ABIs. All ten payload sizes/hashes verified.
ZIP SHA-256 `06ef7841f3d67db7c1751aa172b135e17aa10ac180d86ad40d21d892b83f5c18`.
This is the baseline, not the final modified-code bundle. DDK VFS copies exceeded
32 GiB; six builds used original/flattened image contents, the last used the exact
DDK compiler/prepared tree with its generated include path relocated and native
pahole 1.30 for BTF. No config or validation was disabled. APK signing is still
blocked by the absent production key, and Actions results remain unobserved.

### RFC1 early-loader experiment (production dispatch remains disabled)

The ordinary final ko now advertises preflight protocol 1 in `.modinfo` and takes
an immediate `-EUCLEAN` path before every VPNHide effect. The QEMU harness refuses
to dispatch against legacy/malformed payloads: bounded ELF parsing checks the
exact capability/boolean parameter and absence of constructors; SHA-256 binds the
copied initramfs bytes. A direct syscall oracle preserves errno, unlike insmod's
exit status. CI requires it before real loading; standalone mode needs no network
fixtures. No userspace production preflight loop is enabled by this change.

Actual isolated QEMU acceptance on the pinned android14-6.1 baked kernel
(`6.1.175-g3f3ca1c40482-dirty`): return -1, errno 117; no control/diagnostic nodes,
no resident module, unchanged kprobe list. Summary pass=1/fail=0/panic=0/registered=0.
This proves this early-return experiment on one kernel, not the full vector suite
or other KMI/device safety. ARM64 android16-6.12 compile/final-ELF inspection also
passes; it has no runtime acceptance here. Both artifacts are experimental dirty
builds, not final source-consistent beta distribution artifacts.

Adjacent baseline ELF import comparison found 45 common imports for 5.10 (one
left-only, two right-only), and identical 48 imports for 5.15. All four DDK ko
files have **no classic modversion records**. Missing CRC evidence is not equal
CRC evidence or private-layout compatibility. Never choose the first successful
preflight from an ambiguous set. Remaining: manifest-derived resolution,
durable separate preflight/real-load budgets and crash recovery, the complete
kernel/device acceptance matrix, then explicitly gated production wiring.

Production passive selection now derives every same-LTS candidate from the
verified manifest, carries bounded/anchored Linux-series and release-hint
evidence, and refuses ambiguous or contradictory KMI evidence. A release hint
does not remove the other 5.10/5.15 candidate. Unique catalog series may proceed
through the existing runtime/ownership/full-policy gates; this is catalog
uniqueness, not private ABI proof. Candidate order never authorizes a load.
Resident kmod reconciliation cannot override ambiguous KMI evidence. The full
same-policy alternative selector may choose KPM when it is actually eligible;
otherwise the state remains held. Removed the old exact-uname selector and its
test, replacing it with executable candidate-set/ambiguity/conflict tests.

### RFC1 capability and durable preflight state

Unified assembly now requires protocol-1 capability in every final KMI ELF and
binds it into that payload's existing size/hash metadata. Core independently
parses the bounded final ELF after digest verification; a legacy or inconsistent
ko cannot acquire capability from a producer's version string. Old catalogs
remain readable without capability and cannot authorize preflight.

Private journal is now **7, reading 1–6**. Migration preserves startup/operation
holds, real-load budgets, quarantine and Zygisk exposure; never delete it for a
downgrade. A separate frozen preflight plan binds boot, manifest digest, Linux
series and exact candidate metadata. Each candidate hash can dispatch once;
bundle changes in the same boot do not reset the budget. Running becomes Unknown
on restart. Unknown holds startup, debug capture and explicit native repair, even
after reboot/receipt pruning. Global loader rejection stops subsequent probes.
Only a complete unique matching sentinel may pass the ledger's real-kmod check;
Prepared still supplies the independent policy/runtime/ownership gates. Two
sentinels are explicitly ambiguous. Early-return success is not private-ABI proof.

The fsync-before-effect/after-result executor is tested with injected effects,
including failures before and after each rename; intent failure dispatches zero
syscalls, completion uncertainty retains the Native hold. Production syscall
wiring remains disabled: only the isolated QEMU oracle has run a real preflight.
Explicit post-reboot reconciliation for an unresolved experimental attempt and
the full kernel/device matrix remain required before enabling production probes.

Known conservative recovery boundary: fresh or interrupted 5.10/5.15 selection
with a live kmod still holds when the catalog is ambiguous. A previously journaled
payload hash is intended identity, not an attestation of resident kernel bytes;
this change does not invent that proof from uname or mere residency. Completed
same-boot selections keep the existing durable restore path. A future recovery
adapter may use sufficient independently bound generation evidence; never fix
this by permitting a second load or guessing the branch.

### Public-API Zygisk loader and real Android process prototype

`loader-ipc` owns the private version-1 bounded companion frames, SCM_RIGHTS
ownership, request-time SCM_CREDENTIALS and exact executable-image inspection.
The real V2 `with_companion` handler replaces the empty companion. It duplicates
the actual mapped backing inode during pre-specialization, sends that descriptor
and the public module-directory descriptor, and closes all callback-owned fds
before returning. No dladdr pathname/hash or embedded version is loaded-byte proof.
Core's private root-only observer challenges the companion, verifies both child
and companion mappings against the retained image, seals/root-file protection,
ELF ABI, catalog hash/size, recipient inode, boot and consumed policy generation.
The catalog is verified once per pinned directory generation; subsequent callbacks
recheck its bounded manifest. Hash loops and frame completion have deadlines;
the private Core connect is nonblocking/deadline-bound. The public provider's own
`with_companion` connect remains a provider/device acceptance gate.

Core owns a bounded registry. A pre callback is not specialization proof: Core
joins PID/starttime/UID/process name with actual post-specialization kernel facts.
Availability also requires the original zygote and companion to remain live, the
companion still mapping the retained image and matching current catalog/recipient.
Old boot/epoch, PID reuse, companion replacement and directory replacement do not
reuse a proof. Image inspection uses retained descriptors rather than map_files
privilege assumptions. Unreadable/closed/unsealed provider images remain Unknown.
Positive `ro.zygote=zygote64` plus consistent live zygote inventory removes the
otherwise conservative 32-bit proof requirement. A normal Service cannot select
Zygote32 with `--abi`; mixed runtimes still need genuine 32-bit evidence.

The APK has an unexported, non-isolated `:diagnostics` LoaderProbeService, started
by Core through fixed Android commands after PM readiness, off the command writer.
A short temporary idle allowance is a bounded AOSP launch candidate, not a blanket
background-limit bypass; it expires without removing another owner's allowance.
The dedicated worker terminates itself after a bounded lifetime so the next run
can consume fresh authorization without killing UI. Live workers can be joined.
Bundle assembly installs exact inert autoload copies and activation metadata;
missing/revoked/current-boot-invalid authorization installs no hooks. Actual mapped
image evidence replaces the old forced-Unknown eligibility placeholder.

This is loader availability/consumption plumbing, **not completed stage4 or RFC2**.
It does not prove hooks installed, policy health, game-process updates or fixture
results. Still required: unified typed observed-generation consumer and health
model, lifecycle/fallback completion, full verifier/provider/Android acceptance,
then the autonomous native/framework fixture session and Core-owned verdict/cache.
No device operation or controlled 32-bit package installation was performed.

Cloud validation of this checkpoint: 295 Core library tests and 9 loader-IPC
tests pass, including real sealed memfd mappings, fd cleanup after ancillary
truncation, a real child sender over a parent-created socketpair, saturated
connect backlog, cached generation replacement, PID reuse, companion loss,
required ABI coverage and same-boot restored Zygisk without loader proof.
894 JVM tests, ktlint, detekt, CPD and Android lint pass. Strict host and both
ARM ABI Clippy pass. Pure proc fixtures test the post-specialization registry;
they are not a real Android/provider end-to-end acceptance. Production signing
and final source-consistent release/bundle rebuilds remain separate gates.

### RFC2 native fixture checkpoint — 2026-11-02 08:15 UTC

Implemented a real exclusive, nonpersistent TUN controller and shared bounded
native observations/evaluation. The fixture stays DOWN, has a Core-derived name,
kernel ifindex and explicit MTU, and installs no addresses/routes/rules/traffic.
Negative control, exact raw root stimulus at both ends, protected observations,
ordinary loopback flags/MTU/enumeration preservation and post-close absence are
required. A permission failure or empty enumeration is not successful hiding.
Known leaks survive later probe/cleanup failures; malformed/conflicting dumps,
trailing data after DONE and unknown indices cannot yield proof. Netlink routing
observation now retains ifindex and handles rename/stale deletion by identity.
No broad diagnostic-name prefix is excluded from real VPN observations.

Actual local android14-6.1 QEMU results: RAW sees the addressless DOWN fixture on
flags, MTU, getifaddrs and GETLINK; protected UID10000 under the same final ko
sees ENODEV for both ioctls and omits only the fixture from both enumerations.
Root stimulus remains exact, loopback is preserved, address/route controls are
unchanged and final cleanup is confirmed. Both modes also pass CLOEXEC child-fd,
last-duplicate and abrupt-owner-exit cleanup controls. Complete serial evidence
is retained outside the repo under `vpnhide-build-evidence/rfc2/`.

The experiment caught two real harness/probe defects before passing: the new
getifaddrs collector had discarded non-AF_PACKET/null-address entries; it now
preserves every nonempty name and captures family/flags/null-address metadata.
The raw TUN record is index15, flags4240, families=[], null_address=true. The
restrictive host umask had also made initramfs / and the oracle executable 0700,
blocking UID10000 exec; test paths now have explicit 0755 permissions. The
protected oracle uses generated backend/hook IDs, not guessed backend numbers.

This is **four interface vectors only**, using static musl syscalls and a test
setuid actor for kernel filtering. It is not Android bionic, Zygote, SELinux,
Zygisk hook health, framework/Binder, address/route/rule/SIOCGIFCONF/SO_BINDTODEVICE
coverage, every ordinary network object, or old-game-process acceptance. The
required full plan must not be reduced to this fixture's current reach. Core's
real worker session, complete required plan, cache/fingerprint and unified verdict
remain next; no autonomous suite is claimed ready. The dedicated test oracle is
feature-gated and absent from Core/APK ZIPs. CI wiring adds raw/protected QEMU
controls; those CI executions have not been observed from this cloud session.

### RFC2 Core/Android session checkpoint — 2026-11-02, RPC14

- Added the Core-owned bounded run, full supported diagnostic contract + enabled
  optional features, typed session/task/report/refusals, and typed UniFFI/publication
  adapters. The genuine non-isolated `:diagnostics` Service now joins through its
  device-protected app socket, executes in-process native observations and reports
  them. No Activity/foreign VPN/split-tunnel prerequisite or APK root execution.
- Core keeps actual connection `SO_PEERCRED`, re-resolves app owner, PID starttime,
  exact process name and live root Zygote parent at claim and report. Session,
  epoch, owner, deadline and duplicate-report gates do not trust worker-supplied
  process identity. Work permits remain with physical blocking work after caller
  cancellation. Tests exercise replay/epoch/PID reuse/deadline and typed evidence.
- A diagnostic task is published only after the previous dedicated worker is
  stopped through ActivityManager and its process has disappeared. The Service
  terminates only its own named process on destruction; no whole-APK force-stop
  or arbitrary numeric-PID kill. This prevents reuse of an old Zygisk policy.
- The interface-only fixture is now connected to this real worker adapter. All
  post-creation exits (launch failure, timeout, cancellation and report) finalize
  the owned FD and read back cleanup. Infrastructure records retain scoped native
  evidence and explicit cleanup status; automatic retries require confirmed cleanup
  and are bounded to three attempts per fingerprint. No cleanup uncertainty replay.
- Boot starts a Core supervisor after policy passes. It observes effective state
  changes and bounded transient retries independently of APK opens. The dedicated
  Service is direct-boot aware and uses DE storage; actual OEM/background-start,
  JNA/bionic, SELinux and user-lock acceptance remain device gates.
- Fingerprints include boot/Core epoch, canonical policy, selected/applied runtime,
  delivery/holds/errors, build+Connectivity APEX manifest and actually observed live
  loaded companion/zygote code generations. Child PID/starttime membership is kept
  out of the heavy-run key: UI/game churn does not recreate the fixture. The bounded
  loader history is NOT a complete
  game-process inventory: `loaded_generations_complete=false`; absent platform or
  generation evidence cannot support current reusable proof. Stable partial/error
  receipts are retained stale to avoid repeated work on reads. Successful fresh
  workers do not establish old-game process health. Publication changes atomically
  invalidate relevant earlier evidence; external VPN routing is not a run trigger.
- The only implemented controlled worker slice remains four interface vectors.
  A complete supported plan stays required (addresses, binding/readback, routes,
  rules, proc/sys, framework getters/parcels/callbacks/PendingIntent and optional
  features). Four successes produce `Incomplete`, with the remaining vector list;
  there is no full-PASS path in this partial implementation. Previous completed
  scoped evidence remains retained separately inside Core while the next attempt runs.
- Still partial: real framework fixture/RAW context/recipient callback isolation,
  broader native stimuli, worker loaded-generation attribution, authoritative complete
  process coverage, replacement of the old UI-owned suite and unified UX/export
  consumers. Do not declare RFC2/stage5 finished from this checkpoint.

- Reviewed follow-ups: a pre-specialization child must have a proc birth tick at/after
  this launch barrier; post-allocation setup failures also close/read back cleanup;
  unknown/failed cleanup holds **every** new admission, including force/context
  changes, until exact resource reconciliation (unidentified allocation stays held).
  The fixed stop-service adapter trusts observed process exit even when AOSP's
  stop-success branch returns nonzero. Monitor spawn failure resets its admission flag.
- Final slice checks: 305 Core + 13 RPC + 2 FFI + 1 client + 5 model library tests;
  strict host/ARM64/ARMv7 Clippy and feature-oracle Clippy; 894 JVM tests, ktlint,
  detekt, CPD, lintDebug passed. Fresh QEMU raw/protected runs both passed, including
  newly injected post-TUNSETIFF setup failure cleanup, last-dup and crash-owner
  controls. Serial evidence is in the cloud evidence directory under rfc2/rpc14-*.
  These remain static-musl kernel oracles, not Android acceptance or release bundles.


### Source-bound maintenance and Kotlin migration — RPC15 / journal8

- Core prepares the fixed 0.x source inventory, identity/hash/read completeness
  and profile mapping off the command writer. Interactive PM observation is a
  single bounded pass constrained by the request deadline, not the boot readiness
  loop. Queue timing starts after preparation; cancelled/expired preparation cannot
  admit a later writer. Physical RPC permits survive caller cancellation.
- `MigrateLegacy` carries only mode+base. Rust derives merge/replace/bootstrap,
  preserves enabled hook overrides, auto-hidden marks and mandatory self roles.
  Empty/incomplete Replace and bootstrap over existing user choices refuse before
  Persist. Shared UID observers map to every package; partial profile inventory
  cannot authorize cleanup. APK previews call the same unprivileged Rust policy
  with full package->UID inventory. Kotlin no longer owns legacy conversion or
  privileged cleanup eligibility; the Save/draft UI remains unchanged.
- The source-bound MaintenancePlan is recorded in journal8 before effects and
  follows existing writer/admission/phase recovery. Persist precedes runtime work;
  Cleanup is last. Each unchanged source gets an exclusive/fsynced snapshot and
  no-replace move of its original inode into persistent Core maintenance storage.
  No irreversible unlink: late writes through an old legacy FD remain retained.
  Changed inode/content, unreadable/symlinked source, or redirected archive stays
  preserved. Uncertain move/durability stops further source actions and holds Cleanup.
- Stable migration mode+original base recognizes retained receipts before capture
  or candidate derivation, even after source archival, later edits or Core restart.
  Different requests cannot reuse the ID. Schema1–7 migration preserves all native
  load/preflight budgets, quarantine, Zygisk exposure, receipts and resource holds.
- Domain tests moved to shared Rust; JVM cases now exercise real FFI adapters.
  Replaced the legacy source.contains guard with executable Core root/debug
  collector coverage and added shared-UID/incomplete-profile FFI regression.
- Current checks: 314 Core + 13 RPC + 2 FFI + 1 client + 3 migration-policy library
  tests and strict host Clippy passed; 894 JVM tests, detekt, CPD, ktlint and
  lintDebug passed. Strict ARM64/ARMv7 Clippy and all six process-journal crash tests
  passed too; the schema migration case explicitly covers prior versions 1–7.
- Stage6 is not complete: verified replacement staging, positive legacy module
  identity, manager adapter operations, reboot verification and narrow missing-Core
  bootstrap still need the same maintenance executor. KPatch provisioning remains
  separate and must not be bundled into Core. No phone/root-manager mutation made.

### Native KPM inventory and competing dispatch

- Removed the APatch-directory/Next-CLI shortcut that treated native SukiSU KPM
  as absent. The daemon now runs a bounded, early single-threaded inventory child:
  real KSU driver discovery/GET_INFO, extension ABI2 enumeration and read-only
  VERSION/NUM/LIST; separately, null-key Next HELLO/version/NUM/LIST where no
  managed transport is installed. No load/unload/control is used by this probe.
- Native SukiSU's control ABI has no VPN Hide ctl0 output buffer. An observed
  resident module is incompatible/unmanaged, never a Ready transport. A positively
  identified but unenumerable runtime blocks competing native loads and Zygisk
  publication even without APatch files. CLI inventory requires stable NUM/LIST
  counts, distinct complete names and the exact HELLO echo, not an exit-zero
  negative count, empty output or help text.
- Unknown KSU extension ABI remains Unrecognized, not absent or proven broken KPM.
  It permits retaining only an already observed builtin with no installed/suspected
  KPM; fresh loads and Zygisk publication stay fenced. Restored selection preserves
  this distinction; device capability ownership never becomes Clear from it, and
  ordinary runtime observations stay incomplete. Current Next LIST returns the
  copied buffer extent (4096), while the pinned Suki bridge returns string length;
  those readback checks intentionally differ.
- Checks: 65 activator and 317 Core library tests passed; strict host/ARM64/ARMv7
  Clippy passed. The native driver/syscall adapter is cross-compiled but has no
  Android device acceptance. Generic/old KSU ABI handling is conservative and is
  not proof of universal manager compatibility. Existing keyed APatch transport
  isolation is still a separate implementation concern, not a passed device gate.
- This is an internal observation change: APK/Core remains RPC15, journal8;
  kernel control2 / telemetry1 is unchanged. A user builtin kernel does not need
  rebuilding because of these APK/Core protocol or journal schema numbers.

### Separate provisioner: offline evidence, fork patches and write model

Implemented:

- Separate `vpnhide_provisioning` workspace package, not a Core dependency or ZIP
  payload. The executable exposes only bounded regular-file Image inspection.
  Exact extracted ARM64 bytes/config are hash-bound; gzip CRC/end/size/ambiguity
  checks, explicit KALLSYMS and ALL state, real lowercase Kconfig symbols and
  ARM64 memory-size semantics are covered. Evidence is constructed from bytes,
  not deserialized into execution authority.
- Runtime Absent/Present/Unknown and engine-hash-bound ALL=n qualification are
  distinct. Tool bytes/hash/source commit form a checked tuple. The injected
  write executor requires full-partition backup, bound slot/OTA/snapshot/AVB facts,
  fresh consent, durable intent, post-fsync recheck and independent readback;
  operation reuse and uncertain completion cannot replay a write. BootPlan is
  private/validated and cannot be deserialized into authority.
- Reviewed patches against Module b5612adf4d4085a253cb0be30fa1be36e11d09db and
  Engine 456744b29efb9989445463ab29e368fa59a103c4. Engine config extraction no longer
  follows temp.gz, underflows reversed markers, ignores gzip failure, emits an
  invalid prefix or discards its error result. Module stages only, preserves
  active data, removes legacy autoload/deletion, and fences every legacy direct
  flash/unpatch/partition-guessing entry. An offline builder accepts local
  hash-bound/pinned tools and an exact reviewed module allowlist; no latest/API
  downloads and no production hashes or qualification proofs fabricated.
- Eight Rust tests and strict Clippy passed. Five executable fork tests apply
  patches to pinned source, compile/run the real C parser, exercise invalid gzip,
  temp symlink, tool/allowlist refusal, staging and legacy write refusal. Real
  QEMU Image c0472bd87ccc82dfa5e7d0a18a0333258d39ca1d2b425b8536a754a232cdd3f0
  produced the same 190887-byte config in patched C and Rust (SHA256
  13c82843ae116c94641205624ce17c8f4d75c398959d2b09850f16c25bbdbd30).
  Ruff/actionlint passed; CI includes the new Rust package and pinned fork tests.

Not implemented yet (code, not phone gates): actual provisioner runtime adapter,
trusted tool/qualification record ingestion, fresh boot extraction and offline
patch/repack executor, pinned partition/update exclusion adapter, durable backup
and receipt store, Android consent/progress wiring. The pure write model cannot
flash and does not substitute for these components. Stage7 is partial.

Needs external target/device/key: fork publication namespace/repository/branch
for Module and Engine; actual ALL=n/private ABI/boot qualification and separately
approved real boot writes/readback; production signing key. No repository was
created or published, no boot/phone access and no new permission request blocks
other cloud work. Continue Stage4/RFC2/Kotlin implementation after this slice;
Save/draft remains and final startup/upgrade/downgrade regression is still required.

### Durable authorization and observed process consumption — RPC16 / journal9

Implemented:

- Current Zygisk authority is retained outside the bounded receipt ring. It is the
  latest admitted operation/generation/boot/recipient/wire, not a publication or
  health claim. Schema1–8 migration uses retained execution/admission order, never
  numerical ID order (reserved IDs can execute out of order). Future schema10
  refuses without deleting/resetting the source. Holds, preflight/load budgets,
  quarantine, maintenance and existing exposure evidence survive migration.
- Native lifecycle now retains a bounded exposure boot set. Retiring unknown legacy
  Zygisk is possible exposure too; removing targets is not unloading old hooks.
  Migration unions retained retirement/history/authority evidence even when the
  old marker already names a different boot. Sameboot kernel repair/preflight stays
  blocked after either authorization or possible legacy exposure.
- Typed RPC16/UniFFI/Kotlin read model separates exact authorization readback from
  Current/Stale/Unattributed process consumption, sampled BOOTTIME, incomplete
  process inventory and unmeasured hook health. Bundled startup selection is
  Authorized, including verified restore normalization; explicit legacy module
  compatibility remains Adopted. Snapshot publication fences the exact expected
  authority, so an off-writer old sample cannot replace a newer Save's tuple.
- Consumption revalidates boot/epoch/catalog/recipient, retained image hash, child
  and companion mappings, UID/name, real zygote parent and kernel PID/starttime.
  Identities are rechecked after hashing, with an injected PID-reuse-during-read
  regression. Exited workers can support loader availability, never live coverage.
  Fresh worker consumption never proves that old games updated or hooks installed.
- Loader/diagnostic runs share one dedicated-worker lease. Reuse requires the exact
  expected generation/hash, and fresh-start success must belong to the matching
  new PID/starttime. Fixed stop-service/observed-exit/launch adapters replace the
  separate stale-worker launch path. No privileged descriptors survive app
  specialization. Diagnostic join/report refreshes observed consumption while
  the real worker is alive. Per-process coverage stays outside the heavy-run key.
- Process fixtures now speak actual CLI HELLO and stable NUM/LIST. Prior tests that
  allowed an unenumerable positive KPM to authorize a competing backend were
  corrected to require refusal, with real Core-process/RPC regressions.

Checks: 65 activator +324 Core +14 RPC +2 FFI +1 client library tests; 17 process tests across
journal crash, kernel, KPM and Zygisk publication; strict host/ARM64/ARMv7 Clippy;
894 JVM tests, detekt, CPD, ktlint and lintDebug passed. Disk exhaustion during the
first FFI link was recovered by cleaning reproducible dev outputs and rerunning
with debug data disabled; source and QEMU evidence were preserved. No phone test.

Still code: shared journaled revocation executor and actual bounded fallback,
complete typed attempts/health/coverage consumers, framework fixture and remaining
native stimuli, export/capture/agent ownership transfer, replacement/root-manager
maintenance/bootstrap, concrete provisioner adapters. These are not device gates.
Phone/provider gates remain real injection/Binder/SELinux/background-service and
old-process acceptance, kernel preflight/private ABI/ALL=n/boot evidence; production
signing and fork publication target remain external. Save/draft stays unchanged;
the required final startup regression still runs after the whole architecture work.

## Cloud checkpoint: scoped framework fixture, RPC17

Implemented code (separate from device acceptance):

- Core owns an explicit Prepared → Raw → Protected → Revoked → Finished worker
  protocol, exact retry semantics, fixed lease, root authorization, bounded ACK
  waits, system_server PID/birth/UID/name verification, evaluation and cleanup.
  Source publication is separate from observations and real production filtering.
- The genuine Zygote `:diagnostics` worker performs public ConnectivityManager
  calls on actual opaque Network handles returned by system_server. Its bounded
  partial wake lock covers measurement; framework failure still submits native
  observations. It never computes a protection verdict.
- system_server injects identical dirty VPN and ordinary `ipsec*` objects before
  normal production after-hooks, scoped to the exact authorized outer Binder
  transaction and worker UID/PID/birth. The RAW exemption also covers parcel
  backstops, without changing canonical policy or other processes of that UID.
  Nested untrusted Binder calls clear inherited scope. Inherited onTransact
  methods are deduplicated and each callback owns its restoration key.
- Production branches independently record actual enumeration removal and
  by-handle NC/LP rejection. Clones retain ordinary objects. Prepared/Revoked are
  negative controls on the same handles; Raw supplies positive controls. Old
  system_server loaded-source identity cannot silently match a newer worker.
  The inlined source marker covers hook/generated sources, reviewed shared
  dependencies and build/dependency configuration; it is not a binary signature.
- Scoped results and source/process attribution are typed across RPC/UniFFI.
  Three synchronous framework proofs do not discharge default-network,
  legacy, callback or PendingIntent requirements. Diagnostic contract2 explicitly
  retains default-network work; native4 + framework3 is still incomplete.
- Cleanup terminalizes the run before I/O; an already admitted late step cannot
  recreate authorization. Known cleanup failures retain a retryable exact run.
  Android startup reconciles old Core-epoch leases before launching a new worker.
  Captured, validated leaks survive missing final observations or cleanup errors.

Cleanup confirmation concerns the exact root authorization and matching fixed ACK,
not every inert staging file after a system_server/Core crash. Atomic ACK staging
files contain no authorization. Fresh root-file checks and BOOTTIME expiry are
required for every fixture/RAW decision even if observer delivery is delayed.
Malformed/unrelated authorities remain preserved and block new admissions.

Not implemented yet (cloud code, not device gates): callback/request/default and
PendingIntent/legacy fixtures, remaining native address/routes/rules stimuli,
complete loaded-generation inventory and reusable full-run cache, replacing the
legacy UI suite/domain with this Core run, unified assessment/export/agent views,
Stage4 shared revocation/fallback executor, replacement manager/bootstrap vertical,
and concrete provisioner device adapters. Save/Cancel/footer and picker drafts
remain unchanged; the cancelled autosave proposal must not be revived.

Requires phone separately: actual Binder attachment/source dispatch and production
filtering on Android bionic/Connectivity APEX, system_server SELinux channel access,
negative/shared-UID/process-restart controls, background start/Doze, callbacks and
cleanup under process death. Host model/channel/JVM tests are not those proofs.
Requires signing key separately: final signed production APK. No phone was touched.
RPC17 APK and Core must be paired; private journal9 still reads1–8 and preserves
holds/budgets. Kernel control2/telemetry1 remain unchanged and do not require a
compatible builtin kernel rebuild.

Scoped framework checkpoint validation: 331 Core + 12 diagnostic-model + 14 RPC
+ 3 FFI library tests passed; strict host/all-target and Android ARM64/ARMv7 Clippy
passed. 898 JVM tests, ktlint, detekt, CPD, lintDebug and no-root/scaffold structural
checks passed. Evidence: `/tmp/vpnhide-framework-tests.log`,
`/tmp/vpnhide-framework-{clippy,arm,android}-verified.log`. No device or production
signing test was performed. Final startup acceptance must still be repeated after
the remaining architectural changes.

Stage4 admission correction: the shared journal guard also checks the pending
intent's own legacy/unknown Zygisk retirement, before persisting it. It cannot
admit a kernel load/adoption merely because the previous exposure set was empty.
Observation/retirement remains admissible; explicit Repair cannot override this
boot fence. Eleven journal tests pass, including builtin/ko/KPM and explicit paths.
Shared journaled revocation/fallback executor remains subsequent implementation.

## Cloud checkpoint: shared Zygisk revocation, RPC18 / journal10

Implemented: startup retirement, explicit repair and FullReset call one writer-owned
revocation executor. It pins the retained recipient device/inode, manifest contract
and exact boot/generation/wire grant. Intent and result each sync independently;
absence is read back on the same recipient. A different grant/recipient is never
blindly removed. Disabled/removed modules or missing executable libraries may have
their exact old grant revoked, while publication remains prohibited. Publication
eligibility and removal eligibility are deliberately distinct capabilities.

Revocation is independent of receipt retention and retains the original grant and
boot exposure. Running/Unknown revocation holds Native/preflight and is fenced on
restart. The new typed RPC/FFI status is shared by recovery admission and consumers.
Changes invalidate the publication observation atomically; an offwriter sample
also checks the complete authority/revocation source pair before publication.
Existing process samples remain separate from file publication and hook health.

FullReset withdraws applied/current delivery proof after revoke instead of claiming
its preceding temporary empty publication is still applied. A narrow explicit
Repair is available when config is absent and this exact revocation remains pending:
no canonical creation/revision update, startup selection, grant publication or
unrelated hold supersession. Its Native receipt is NOT Native apply evidence.
An old revoked Zygisk grant cannot withdraw a newer builtin/kernel delivery.

Narrow admission and grant association are one durable write. Repeated repairs
transfer only the prior narrow receipt's hold for that exact grant to the new
Running intent in that same write; historical Unknown remains immutable. The
pending narrow receipt is pinned through pruning, including retention0. Durable
subeffect completion can resolve a not-yet-finalized narrow receipt after a crash.
This avoids admission, retry-chain and completion-window orphan holds.

Private schema10 reads1–9 without resetting holds/budgets/grants; unknown schema11
is rejected with its source preserved. RPC18 APK/Core must be paired. These versions
remain independent of unchanged kernel control2/telemetry1; compatible builtin
kernels need no rebuild merely for an app/Core upgrade.

Still unimplemented cloud code: shared whole-attempt/frozen-candidate executor and
known-no-effect bounded fallback, unified assessment/UI/export/agent integration,
remaining framework/native diagnostics stimuli, replacement manager/bootstrap and
concrete provisioning adapters. Those are not device gates. Device acceptance of
real loader/Java hooks, final same-commit signed APK/bundle, fork publication target
and actual boot writes retain their separately documented requirements.

Revocation checkpoint validation: 342 Core + 15 RPC + 3 FFI library tests;
18 actual-process RPC tests across journal crash, kernel, KPM and Zygisk passed.
The crash scenario includes paired admission, intent, unlink, completion and a
second completion crash during retry, keeping historical Unknown immutable and
leaving no orphan Native hold. Unit write-fault recovery also covers retention0.
Strict host/all-target and ARM64/ARMv7 Clippy passed; 898 JVM tests, ktlint, detekt,
CPD and lintDebug passed. Logs: `/tmp/vpnhide-revocation-{tests,process-final,
clippy-verified,arm-verified,android}.log`. These are cloud checks; device loader,
framework and real background lifecycle acceptance remain unclaimed.


## Cloud checkpoint in progress: frozen attempts and no-effect fallback

Implemented cloud code: startup and Repair now share the writer-owned bounded
attempt executor. One verified catalog and one UID inventory freeze full candidate
projections and anonymous payload descriptors before dispatch. Every candidate
gets a durable startup intent, and Repair updates its exact Native recipient/wire
in the same write. Completion/proof must sync before any successor. Unknown,
timeout, resident partial health and Java/Ports failures never advance the chain.
Fresh ownership and pinned directory/catalog generation are checked again after
intent/retirement, before every dispatch. A KPM appearing during intent persistence
prevents the first ko syscall too. Frozen Zygisk recipient identity survives direct
boot selection, restore and the later policy authorization, including root replacement.

No-effect evidence is private and separate from the public signature diagnostic.
Actual Android finit_module errno126/129 requires final hash-bound init_contract1;
host process statuses/text never qualify. Audited init normalizes all other returns
away from those signature errors. ELF validation rejects constructors, own setters,
metadata aliases, swapped bool/uint operations, scalar OOB/misalignment and other
objects. Rust/Python share valid and five malformed binary fixtures. Typed KPM
before-dispatch failure is distinct from every ambiguous post-dispatch result.
Proof binds boot, payload hash/size/capability, manifest/source and fresh ownership.
Budgets are never refunded. Restart can skip a spent candidate only with retained
valid proof for the same generation/policy and fresh ownership; unknown holds remain.

An independently retained prior exact Zygisk grant survives repeated newer admitted
but unpublished generations. Reset/retirement can revoke that known old grant;
unknown or unexpected newer files remain refused. Exposure is retained, never
interpreted as hook unload. Save/Cancel/footer/draft UX stays unchanged.

Current compatibility: RPC18 unchanged; private journal11 reads1–10. Older Core
must reject schema11, preserving the file. Kernel control2/telemetry1 unchanged;
no builtin rebuild is required merely because private/RPC versions changed.

Validation: 352 Core library tests and all process RPC tests passed; strict host
and ARM64/ARMv7 Clippy passed. 15 Python ELF/bundle tests, Ruff, C exhaustive
return-normalization test, shellcheck/actionlint and formatting passed. New fault
tests cover every per-attempt intent/completion boundary, no successor after
Unknown, same-policy restart after rejection, first-load ownership races, frozen
Zygisk recipient replacement and retained prior-grant revocation after 36 failed
admissions. A real QEMU6.1 ko
with preflight1/init_contract1 passed both ELF inspectors after rebuilding; unsigned
SHA0643eb4b74bf198f60dd3b45a9ff05b0a23a8530de881347f48c097f003d165c.
Separate signature-enforced Linux6.1.175 QEMU experiment PASSED: exact errno129
for unsigned and modified-signed copies; no proc nodes/module or changed kprobes
after either rejection. The exact same unsigned ELF plus a test-kernel signature
returned117 for preflight without effects, then its single real load returned0
with hooks0x20003ff/error0. Reproducible harness: `kmod/test/run-signature.sh` and
`signature-probe.c`. Evidence: `/workspace/vpnhide-build-evidence/fallback-init-contract/
signature-evidence.json`, `signature-qemu/{serial.log,payloads.sha256,kernel.sha256}`.
The locally generated key belongs solely to that isolated test kernel and is not
published or used for APK/production signing. This is not an Android/device or
all-kernel proof.
Seven final KMI builds, final paired APK/full bundle and device acceptance remain
open. Production signing key is unavailable. No phone/boot writes were attempted.

Still unimplemented cloud code: unified assessment/UI/export/agent integration,
remaining framework/native stimuli, replacing the legacy app-triggered diagnostic
suite with accepted Core runs, replacement manager/bootstrap, concrete provisioner
adapters. These remain implementation work, not device gates. Final startup
regression for `core root snapshot probe exit=-1` must follow those remaining edits.


## Cloud checkpoint: Core-owned autonomous assessment foundation, RPC19

Implemented (not yet production consumer cutover): one Rust `DiagnosticRun::assessment`
projects supported required checks, scoped proven/leaked/unverified checks, native
and framework controls, execution phase and reuse blockers. It preserves nested
native leaks even under Unverified/Infrastructure/cleanup failure, and framework
leaks even with invalid controls. There is deliberately no full-success variant.
Seven implemented scoped vectors cannot shrink the nineteen-vector base contract.
Core computes and publishes this exact assessment atomically with its retained run,
including staleness/invalidation updates; UniFFI/Kotlin only transport it. RPC19
adds this paired read model. Private journal11/kernel control2 remain unchanged.
Kotlin does not reconstruct a verdict from counters or clean Java values.

Core's `stale` now means actual fingerprint/context change. Unknown platform or
incomplete loaded-game generations remain explicit reuse blockers, not invented
condition changes. Retaining partial/infrastructure results still launches no
heavy suite on reads. Diagnostic RPC transport wrappers now use the same
connection-generation failure owner as probes/commands; refusal and waiter
cancellation leave that connection intact, and late failures cannot invalidate a
newer connection. The explicit Core request reply remains a session handle,
not a competing UI publication authority.

A new read-only snapshot owner accepts Core publications, config actor acceptance
and root observations in one immutable packet. It checks epoch, exact accepted
sequence and matching root connection/runtime state, while ignoring only the run
publication cursor when joining root facts. A same-config backend/apply change
cannot borrow older root facts. Disconnect releases a pending root checkpoint
with qualified unavailable retained evidence; a reader cancelling does not cancel
the owner. Construction and reads contain no diagnostic admission path.

Still not implemented: replacing production DiagnosticsCache/DiagnosticDomain,
Dashboard/DiagnosticsScreen, agent and export together. The old local suite is
STILL wired in production at this checkpoint. This foundation must not be reported
as completed ownership migration. Preserve Save/Cancel/footer/drafts. Remaining
native/framework stimuli, full-generation proof, maintenance/bootstrap and concrete
provisioner adapters also remain cloud implementation work. Final screenshot startup
regression and device acceptance are still outstanding after that work.


Foundation validation: 353 Core + 14 diagnostic-model + 15 RPC + 3 FFI library
tests and all61 process RPC tests passed on RPC19. Strict host/all-target and
ARM64/ARMv7 Clippy passed. 907 JVM tests plus ktlint/detekt/CPD/lintDebug passed;
the final nullability cleanup was rechecked with all9 new snapshot/transport tests
and Kotlin quality gates. No-root/module-scaffold checks passed. Logs:
`/tmp/vpnhide-autonomous-rpc19-{tests,clippy,arm,process,android,android-cleanup}.log`.
RPC19 APK/Core must be paired; private journal11 reads1–10 and kernel control2 /
telemetry1 do not change. Old phone APK/Core045b58a remain untouched.

## Cloud continuation: autonomous consumer cutover (RPC19 / journal11)

Production DiagnosticsCache is now a read-only Core subscription plus explicit
retry admission. Dashboard, Diagnostics screen, agent state, debug export and
logcat metadata share the accepted autonomous packet; startup/tab changes,
config edits and export do not launch the old main-app suite. Retry coalesces
pending taps on the process owner and ignores reply data as a publication source.
Diagnostic transport failure still withdraws only the affected connection.

The UI presents scoped Core required/proven/leaked/unverified evidence, cleanup,
infrastructure and process-generation limitations; partial proof is never green.
The foreign-VPN/split-tunnel prerequisite and capture warning are removed from
these production paths. Save/Cancel/footer and app-picker drafts remain intact.

Bundle schema4 adds the retained Core run/assessment and exact Core/config/root
source identities. Legacy report/gate/selfTestRunId/diagnostics/verdict fields are
null in these paths. The canonical config comes from that accepted current Core
publication even if the config actor has not caught up; an offline publication
cannot borrow canonical text from raw metadata. Root join compares the consumed
canonical/backend/native-boot identity, not unrelated routing/receipt updates.
Dashboard status stays visible while root metadata is unavailable; real load
errors remain errors. Cached Dashboard data does not regress startup readiness.
Unjoined exports withdraw runtime liveness and active statistics but retain raw
forensics and installation metadata. Optional root failure no longer prevents
agent Core state reads, and cancellation remains cancellation. No fresh
post-debug-baseline measurement is claimed.

Not yet removed in this checkpoint: unreachable old DiagnosticDomain/coordinator
and their old presentation/test helpers. Their retirement and transfer of useful
regressions is a following code step, not completed by disconnecting production.
Core base plan remains nineteen vectors; expanded native/framework stimuli,
full game-generation proof, remaining maintenance/bootstrap, provisioner adapters
and the final kernel/bundle matrix remain implementation work. Phone acceptance,
production signing key and fork publication target are separate external gates.
The final screenshot startup regression must still run after subsequent changes.

Consumer validation: 916 JVM tests passed, including zero-admission on reads,
process-owned/coalesced Retry, refusal/cancellation, epoch/config/root acceptance,
unrelated routing/receipt liveness, immutable transport copies, retained export
provenance and current canonical source. ktlint, detekt, CPD and lintDebug passed;
69 unused retired-screen resource names were removed across all three locales,
not suppressed. No-root and module-scaffold checks passed. Read-only reviewer
identified the agent metadata failure/statistics gaps; both were corrected and
rechecked. Log: `/tmp/vpnhide-consumer-tests-final.log`.

## Cloud continuation: retire main-app diagnostic execution

Removed the unreachable AppDiagnosticRunIo, DiagnosticDomain, identified local
run executor, snapshot owner and snapshot assembly, plus capture-to-legacy-selftest
mapping. Removed the legacy-only export recommendation/snapshot summary overloads.
The old report DTO/pure presentation compatibility helpers remain separately;
there is no local coroutine executor or capture admission path behind them.

Test migration/deletion ledger for this cut:

- 35 tests of the retired executor/domain/capture mapping are removed with that
  implementation, not weakened. Foreign-VPN eligibility/confirmation rules and
  capture-owned ROUTED claims no longer describe the accepted product contract.
- RootProjection's observation-ID plus generation regression is moved unchanged
  in behavior to AutonomousDiagnosticSnapshotTest. New acceptance tests cover
  superseded root checkpoints, reader cancellation, already-published source
  acceptance, transport-record immutability and config/epoch joins.
- Core autonomous coordinator tests own deadline/epoch/process-birth/replay fences,
  failed dispatch settlement, cleanup holds, fingerprint invalidation and cache
  no-restart. Diagnostic-model assessment tests retain partial leaks under
  unavailable controls/cleanup and never shrink the supported required plan.
- AutonomousDiagnosticsTest owns process-scope/coalesced explicit Retry and
  zero-admission on startup/read/observation changes, with refusal not retried.
  AutonomousExportTest/schema4 golden own retained-evidence provenance and null
  legacy claims. Existing reducer/security/FFI/migration/probe behavior tests are
  retained; no test target or quality rule was disabled.

Retirement validation: 882 JVM tests passed (35 obsolete-owner tests removed,
one root-projection regression moved); ktlint/detekt/CPD/lintDebug passed on the
final cut. Verification log: `/tmp/vpnhide-retire-local-diagnostics-verified.log`.
No user-visible behavior beyond the preceding consumer cutover; internal removal
needs no additional changelog fragment. RPC19/journal11/kernel2 unchanged.


## Cloud continuation: legacy framework sources and observations, RPC20

Implemented: two distinct authorized Binder scenarios for legacy TYPE_VPN and
NetworkInfo enumeration. System-server fixture sources inject connected, tagged
VPN clones in both RAW/protected phases; enumeration retains original objects
and appends a tagged ordinary sentinel. It does not synthesize a disconnected
answer or replace the platform factory/filter. The worker performs real public
ConnectivityManager calls. Each field reports bounded observations/errors;
Rust owns controls, proof, leaks and the aggregate required plan.

Ordinary preservation uses a bounded canonical getter-field multiset hash/count
from each invocation before production sanitization and from the worker's reply.
Duplicates and ordinary sentinel are retained; real network churn between RAW
and protected is allowed when each invocation preserves its own baseline.
Hashing the expected ordinary objects never invokes their parcel sanitizer.
Source evidence republishes the latest invocation baseline even when its source
flag already exists. Production telemetry records replacement of the tagged
*element*, so normalizing a neighboring VPN or only a later parcel backstop
cannot impersonate that result-hook path.

Prepared/revoked controls require fixture markers absent, not absence of a real
VPN. RAW requires the connected tagged source. Protected requires preserved VPN
type with platform DISCONNECTED/BLOCKED state and no fixture marker. Null, Wi-Fi
replacement, missing source/normalization or mutated ordinary objects cannot
prove success. Disconnected replies retaining the tun marker are still leaks.
Both legacy scenario proofs are required to discharge the legacy type/enumeration
family; either scenario's leak is retained under incomplete cleanup/worker death.
By-handle/default/active/callback/PendingIntent scenarios remain separate and open.

Wire changes are synchronized at RPC20 / framework fixture protocol2. Private
journal11 and kernel control2/telemetry1 remain independent; no compatible builtin
kernel rebuild is required for the APK/Core change. Shared legacy observation
adapters are in the reviewed common source layer and included in loaded hook
source fingerprints. A mismatching old system-server build cannot prove the new
fixture. No phone, production signing key, boot write or fork publication used.

Invocation freshness was tightened during review: the fixture records one Binder
entry per legacy API and phase, clears that API's old normalization/baseline before
it runs, and publishes immutable snapshots. Duplicate calls saturate to ambiguous
and cannot prove protection. Each worker call carries its own CLOCK_BOOTTIME
nanosecond window; server entry time must fall inside it in Prepared, RAW,
protected and Revoked. Missing/mismatched windows cannot borrow an older ACK.
Publication/authority failure at entry becomes an error for the exact diagnostic
worker rather than silently retaining a normalized ACK. Independently attributed
positive leaks survive duplicate-call ambiguity, cleanup failure and worker death.
Regressions cover each negative phase/API independently, retained tun markers,
null/Wi-Fi answers, missing production branches, ordinary mutation/cardinality,
repeated baselines, neighboring-VPN normalization and duplicate ordinary entries.

Legacy validation so far: 353 Core + 21 diagnostic-model + 15 RPC + 3 FFI unit
tests; host/all-target and ARM64/ARMv7 strict Clippy; 888 JVM tests, ktlint, detekt,
CPD and lintDebug passed. All61 host process RPC regressions also passed on the final code. No-root and
module-scaffold checks passed. Logs:
`/tmp/vpnhide-legacy-{rust,clippy,arm,process}-final-v2.log` and
`/tmp/vpnhide-legacy-android-final-v3.log`. No phone acceptance is claimed.


## Cloud continuation: native read model and retirement of VPN gates, RPC21

Implemented code: native selected/applied/observed/displayed
identity, immutable bundled/source/payload provenance, durable attempts, latest
Native phase/delivery, independent desired hook coverage, setup and filesystem
state. Rust owns this projection at every publication boundary. Kotlin Dashboard,
picker, filesystem settings, Statistics, agent and schema5 exports consume it.
Historical raw counter captures retain their identity and unsigned delta/reset
semantics; current rows require Core epoch/config/connection plus boot, implementation,
delivery/runtime identity. Java currentness also requires a current-boot heartbeat.
No global Zygisk process coverage is asserted.

Read-only review found and code/tests corrected: observed-only runtime borrowing
an old bundled hash; failed same-revision preparation reviving an older Confirmed
delivery; stale Java/current counter marks; kernel+KPM coexistence being reported
as unavailable; valid inventory-only KPM absence losing builtin observations.
Delivery now retains the newest attempted phase even without an intent, with
unknown backend/wire identity. Typed runtime conflict survives publication.

Removed code: RoutingGateCache; GroundTruthProbe/JavaChecks/main-process NativeProbe;
DiagnosticGate/DiagnosticEligibility/Context/Presentation/Report/LayerStatus/
CheckOutcome/Situation and obsolete run/confirmation/impact/summary classifiers;
ProtectionVerdict, HeroVisual, VpnOffPrompt and unreachable automatic DonatePrompt;
checks-jni crate and packaging/CI/release entries; Core framework_vpn/routing_compose/
vpn_networks and app-vpn/routing probe dispatch; route/rule listener, framework VPN
polling, UID membership classifier and routing RPC/FFI/app models. Removed native
Kotlin directory-priority selection, manual alternative-KMI recommendation and
Kmod/KPM load classifiers, old JSON boot codec and filesystem classifier.
Core boot DTO mapping, production hooks, real worker APIs, explicit manual
NetworkViewCapture and forensic hook/counter adapters remain.

User acceptance: preserve picker Save/Cancel/footer and drafts. Remove foreign
VPN-active/APK-in-tunnel admission, prompts and listeners entirely, including dead
resources in all locales. Generic Retry is renamed from vpn_off_retry to retry.
Built-in help documents autonomous evidence rather than directing the user to
turn on VPN or add the APK to a tunnel. Actual device gates are still open.

Remaining cloud implementation is not a device gate: expanded RFC2 stimuli and
callbacks; common maintenance staging/verification/bootstrap/UI; concrete offline
provisioner adapter; remaining config import/auto-hide/capture/agent domain moves;
full final kernel/Core/bundle matrix and final startup regression. Existing
0.x/1.x import adapters and journal schemas1–10 are intentional migration readers.
Production APK key, fork publication target and real phone acceptance are separate
external requirements. Do not stop at this cleanup checkpoint.

Validation of this cleanup: 577 JVM tests, ktlint, detekt, CPD and lintDebug passed.
72 unused base resource names and all existing translations removed (58 in each
localized file); generic Retry is renamed, not removed.
The reduction from 888 previous JVM tests accompanies deletion of the retired
VPN-gated report/context/presentation and native-selection owners, not relaxed
assertions or quality thresholds. LSPosed/Ports/environment/migration/counter
regressions were preserved; new tests cover native transport binding/immutability,
same-revision delivery identity, stale Java statistics, config content equality
and Dashboard/autonomous independence of external VPN and split-tunnel state.
Rust validation: 345 Core + 66 activator + 14 checks + 15 RPC + 3 FFI + 7 native-policy
unit tests passed; all61 host Core process regressions passed. Strict host/all-target
and ARM64/ARMv7 Clippy passed, as did fmt/no-root/module-scaffold checks. Logs:
`/tmp/vpnhide-cleanup-{rust-tests2,process2,clippy-host2,clippy-arm1}.log` and
`/tmp/vpnhide-cleanup-android4.log`. No phone acceptance is claimed.


## Cloud continuation: independent socket binding actors, RPC22

Implemented: fresh CLOEXEC datagram sockets observe SO_BINDTODEVICE before and
again after every setter, including a failing setter. libc and direct-syscall
actors each have independent fixture and ordinary loopback controls. Neither
permission errors nor missing readback prove hiding; mutation followed by an error
still retains a positively identified leak. Root negative/start/end and cleanup
controls bind the exact session interface. Worker and Core network namespaces
must match by live procfs device/inode identity, in addition to process birth,
boot, deadline and operation fences. No real VPN, address or route is changed.

Diagnostic contract3 adds typed actor observations and aggregate proof/leak/control
evidence through Rust RPC22, generated FFI, immutable Kotlin state and schema5
exports. Journal11 (reads1–10), kernel control2 and telemetry1 remain unchanged.
Both actors must prove protection before this required vector is discharged;
missing late cleanup or end evidence cannot erase an already confirmed leak.
Manual forensic bind checking also stops treating permission/unknown readback as
PASS or assigning a SELinux cause without evidence. Autonomous runs use their
actual session interface, not a fixed tun0. Nineteen required vectors are retained.

QEMU evidence: the bounded native-fixture RAW and protected modes both passed on
Linux6.1.175 with the earlier verified closed-init payload. RAW exposed the exact
TUN through both bind actors; protected returned ENODEV with empty readback while
ordinary lo remained bindable. The existing four interface vectors, DOWN fixture
route preservation, dup-FD/crash and partial setup cleanup controls also passed.
An earlier invocation used the wrong mode environment variable and instead ran
the general harness without its optional probe/iproute inputs: its 24 failures
are preserved and are not counted as a passed general QEMU matrix. This is static
libc kernel evidence, not Android bionic/SELinux or same-final-commit bundle proof.

Remaining RFC2 code includes address/route/rule/proc/sys stimuli, active/default
framework and callback/PendingIntent controls, plus loaded game-process coverage.
Stage6, concrete provisioning adapters, remaining Kotlin domain transfers and
final complete packaging/startup gates remain implementation work; phone, signing
key and fork target are separate external gates. Picker Save/Cancel is preserved.

Binding validation: 346 Core + 28 diagnostic-model + 17 checks unit tests passed;
all61 Core process regressions passed on RPC22. Typed RPC/FFI units passed; strict
host/all-target, fixture-oracle and ARM64/ARMv7 Clippy passed. The full Android
unit/ktlint/detekt/CPD/lintDebug gate passed, including immutable actor mapping;
schema5 golden passed independently without UPDATE_GOLDEN. Read-only review found
no further concrete bind proof bug. Evidence logs and both bounded QEMU serial
captures are retained at `/workspace/vpnhide-build-evidence/rfc2/socket-binding`.
No phone acceptance or final complete kernel/package matrix is claimed.


## Cloud continuation: publisher-bound replacement preparation

Implemented preparation API, not yet exposed over RPC: a shared package-trust
library verifies exact domain-separated descriptors with the existing Android
key's compiled public identity. Positive compiled CI sequence and persistent
accepted floor are required; unsigned/self-declared identities, modified bytes,
wrong target, incompatible journal/RPC and rollback fail closed. Descriptor binds
post-customize daemon hashes and payload manifest, not manager-mutated module.prop.
CI now orders public export → Core compilation → complete bundle → detached
signature → signed APK target asset. PRs get an explicitly unconfigured trust
store without secrets. The signed descriptor is included beside the ZIP artifact
and in the future release asset list; no release was run. The nonexistent Core
updateJson URL was removed. No production key was read or generated in cloud.

Root preparation copies only a hash-named fixed app input into private storage
beside the journal, outside the running module. Exclusive cross-instance lease,
nonblocking/no-follow/regular-file/owner/link checks, source identity fencing,
rename-NOREPLACE, synced private readback and retained CLOEXEC FD are implemented.
Two final packages bound storage; only inert interrupted copy files are recovered.
No manager action, active replacement, old-module removal or boot write occurs.

Read-only review identified missing release sidecar and a partially configured
public pin without compiled sequence; both were fixed before integration. The
[package preparation note](core-package-maintenance.md) describes this boundary.
Next code remains common durable maintenance state/floor, manager lifecycle,
reboot-before-startup reconciliation and app/bootstrap flow. These are not device
gates. Expanded RFC2, provisioning adapters, Kotlin domain moves, final complete
packaging matrix and exact startup screenshot regression also remain open.

Preparation validation: all355 Core unit tests (nine real filesystem/lease/budget/
crash-copy regressions), four signature/admission units, actual Java-keystore→Rust
verification for RSA/P256/P384, five metadata negative tests and five signing
failure tests passed. Strict host/all-target/fixture-oracle and both Android ARM
ABI Clippy, fmt, ruff, no-root and module-scaffold checks passed. CI YAML dependency
graph parses without missing jobs/cycles; live Actions success is unobserved and
the forbidden API was not retried. Evidence is retained in
`/workspace/vpnhide-build-evidence/maintenance-preparation`. No new APK/Core RPC or
journal format was introduced by this internal preparation cut.


## README and public distribution correction

User review found real stale distribution policy as well as stale prose: all
three top-level READMEs still required separate backend modules/manual GKI, and
release.files still exposed producer ZIPs. The standalone activator executables
and their backend boot/uninstall scripts were already removed; crates/activator
is a Core-linked library, not an independently launched process. Backend inputs
can contain installer/recovery shims, so the old blanket "no scripts" was wrong.

Corrected RU/EN/ZH installation and source-build descriptions, component README
install sections, and marked the cutover note historical with its old separate-ZIP
upgrade checklist withdrawn. Public distribution now uses one executable explicit
allowlist: matching APK, complete Core ZIP and signed descriptor. Backend ZIPs,
daemon-base and arbitrary extra APKs remain internal, including when present in
the same artifact directory. CI runs its behavior tests. No tag or release was
created, and incomplete maintenance/RFC/device acceptance is not presented as done.


## Core-only update feeds and host-tooling audit

Follow-up distribution audit found another live producer policy: update-json.sh
still generated new-version URLs for unpublished standalone backend ZIPs, kmod
stamped an unconditional legacy feed, and KPM/Zygisk inherited UPDATE_JSON_URL.
The generator now emits only update-core.json. Existing legacy JSON files remain
frozen compatibility data for their old installed identities, without redirection
to Core or updates to nonexistent 2.0 assets. Internal producer packaging strips
old updateJson template fields and ignores that retired env input; CI no longer
passes it. Maintainer release docs and update-json/README state this explicitly.
Three behavior tests run the shell generator in an isolated fixture and actual
producer ZIP packaging with compiler commands stubbed; they verify the frozen
feeds and generated public asset, and all three producer metadata results.

The user separately requested only an audit of shell/Python/dev candidates for
Rust. [Host tooling audit](host-tooling-audit.md) records candidates and boundaries;
no broad tool rewrite or device cleanup was performed. Previously approved Core
runtime/provisioner work remains independent of these new audit recommendations.
