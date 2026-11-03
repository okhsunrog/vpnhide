# RFC 0005: Unified backend selection and lifecycle policy

Status: **Proposed — awaiting maintainer review. Not implementation approval.**\
Research baseline: `5a14350b988d6a7e37635f324eb517dcf30a838f`, 2 November 2026.\
Scope: Core-owned backend selection, health, bounded recovery, Zygisk discovery, maintenance, migration, and user attention.\
Document identity: RFC 0005, a consolidated proposal filed for review.

This RFC defines one policy for a user who wants protection to work without learning the differences between a built-in driver, a kernel module, KPM, and Zygisk. **Auto is the default: Core chooses and safely recovers the implementation, including when protection is only partial. Manual is an advanced override: Core uses the explicitly requested backend, permits a safe partial result, and never silently substitutes another backend. Neither mode bypasses compatibility, authenticity, ownership, or nonoverlap rules.**

The design preserves a working backend during long uptime, prepares changes before an already-needed reboot, and treats a reboot as a user action rather than an automatic recovery primitive. It does not promise that every fallback is immediate or that every update needs a reboot. It distinguishes a passive Zygisk mapping from permission to install hooks, and distinguishes a successful load from demonstrated protection.

The request to write or commit this RFC does not authorize its implementation, a release, changes to a device or root manager, installation or flashing, or disruptive testing. Existing separately approved implementation work is not evidence that every new detail below is approved.

## 1. Decisions and proposal boundaries

### Agreed requirements

- Auto makes ordinary recovery decisions, including partial coverage, without asking “KPM or Zygisk?”. Manual selects one backend with no hidden fallback; safe partial protection is allowed.
- Core owns all domain policy and durable state: selection, health interpretation, recovery, maintenance, migration, update scheduling and attention. APK/Kotlin renders typed state and submits intent; it is not a second policy engine.
- Users install one Core ZIP containing one passive Zygisk ABI pair. Archive deduplication permits a verified ordinary installed discovery copy. Main Core has no top-level `zygisk/`; its owned projection has no separate activator, daemon, selector, update channel or user ZIP.
- Neither mode bypasses authenticity, compatibility, authentication, capacity, ownership, holds or nonoverlap. Unknown effects are not a safe failed attempt.
- Core never automatically reboots, restarts Zygote/provider, kills shared companions, invokes unsafe global refresh, or reenables user-disabled components.
- Auto can prepare authorized safe fallback. Significant root/kernel provisioning, boot-image patching/flashing and installing a runtime require separate plan-bound consent. Preparation is not permission to execute those actions.
- Stage known changes before an already-required boot. Compatible APK-only updates need no Core/backend device reboot; two boots are not a patch/minor update rule.
- The approved notification bridge is a private explicit root/Core wake hint → fresh Core RPC state → APK Android notifications, never popups or a second owner.

### Proposed details

The feature-based algorithm, finite recovery budgets, sticky publication default, first-install prepublication, schema/state names, manager/bootstrap adapters and implementation phases below require acceptance. This draft is not approval to implement them. Section 15 gives recommended resolutions rather than putting routine backend choices back to users.

### Existing RFCs

[RFC 0001](0001-automatic-gki-kmi-selection.md) supplies GKI/KMI and bounded safe preflight evidence; no guessing or repeated real `.ko` loads. [RFC 0002](0002-autonomous-diagnostics.md) supplies reproducible behavioral evidence; unimplemented checks remain unverified. [RFC 0003](0003-conditional-zygisk-publication.md) supplies passive packaging, Core-owned discovery and durable/provider boundaries.

**RFC 0003 needs revision before adopting this default.** Its present requirement to exclude discovery before a kernel boot is stronger than this proposal. Passive publication/mapping may coexist with kernel protection when no current-boot Zygisk production grant could have been consumed. Possible hook consumption still blocks switching. Strict zero-mapping is a separate property. Preserve existing conservative guards until the replacement proof model and old-state migration are reviewed; do not silently reinterpret the accepted RFC. [Pinned RFC 0003][rfc3]

## 2. What the baseline actually implements

These facts are pinned to `5a14350b`, not claims about later code or device acceptance:

| Area | Existing behavior and remaining gap |
|---|---|
| Selection | Fixed `Builtin → Kmod → Kpm → Zygisk`; resident reconciliation rather than displacement. No Auto/Manual or health-driven policy |
| Eligibility | Verified payload, architecture/KMI, runtime, projected policy, capacity and required hooks; no consistent feature-granular partial contract |
| Fallback | Only a durable exact no-effect rejection permits the next candidate; not a diagnostic-failure fallback |
| Diagnostics | Required/proven/leaked/unverified checks, controls, fingerprint invalidation, caching and bounded infrastructure retry; no complete all-process proof or selection policy |
| Packaging | One passive ABI pair; main Core discovery mirrors removed and rejected by replacement validation. Projection identity split, manager staging, next-boot planner and RPC/UI remain unfinished |
| Maintenance | Authenticated replacement preparation and journal primitives, not a completed manager/reboot/bootstrap flow |
| User settings | Per-app Native/Java/Apps/Ports and hook choices; no backend preference or Auto/Manual |
| Updates | Daily APK WorkManager check; approved Core scheduling/attention integration not implemented |

Sources: [selector][selection], [preparation][bundled], [transition][transition], [configuration][config], [ledger][ledger], [maintenance][maintenance], [update worker][update-worker].

Important inconsistencies and limits:

- Resident `status.error != 0` currently yields a zero support mask, while KPM delivery accepts `PartialHooks`. The new policy must reconcile startup, delivery and health predicates rather than merely relabel warnings. [Preparation][bundled], [KPM][kpm]
- `DiagnosticOutcome` contains only `Running`, `Incomplete`, `Infrastructure`; an assessment may have scoped proof/leaks without a generic full PASS. `loaded_generations_complete` and `process_coverage_complete` are false. [Model][diagnostic-run], [assessment][assessment], [coordinator][coordinator], [native status][native-status]
- Existing validity includes boot/Core/policy/runtime, platform/APEX and loaded-generation context. Avoiding an identical expensive rerun does not upgrade incomplete evidence to reusable proof. APK reads do not authorize a suite or switch. [Coordinator][coordinator], [Kotlin join][kotlin-snapshot]

Only passive packaging has landed in the conditional-discovery vertical. Missing projection/planner/UI code is an implementation gap, not merely a device-test gate. [Ledger][ledger]

## 3. Ownership and state model

Core's pure policy reducer interprets requirements and evidence. Its existing single-writer command engine/journal admits effects; maintenance performs authenticated preparation and supported manager staging; backend/provider adapters supply exact evidence, not product decisions. Diagnostics produce scoped observations. APK submits intent and handles Android presentation/permission mechanics. The root manager supplies its installation contract.

Keep these dimensions separate:

| State | Meaning |
|---|---|
| Desired protection | Canonical features, targets, profiles and scopes |
| Requested mode/backend | Auto or an explicit Manual family |
| Recommended backend | Core's permissible recommendation, not an effect |
| Effective backend | Reconciled actual owner in this boot, possibly still different from a pending request |
| Next-boot intent | Exact staged target/discovery and prerequisites for a future verified boundary |
| Discovery | Disk/staged projection, provider catalog and retained image per ABI |
| Hook authority and consumers | Grants, possible consumption, revocation and old process/companion/zygote generations |
| Last success | Scoped historical evidence and planning hint, never current authority |

Files prepared, staging accepted, boot observed, image accepted/loaded, policy delivered and protection demonstrated cannot collapse into `active: true`.

Bind evidence to relevant boot, Core contract/instance, config revision/hash, package/manifest/payload, kernel, provider incarnation, ABI and PID plus start time. Platform/APEX and loaded-code identities affect diagnostic validity. Last success records *what* succeeded and its scope; load success is not all-game protection. Revalidate after OTA or provisioning change. A compatible builtin need not be rebuilt because APK RPC/private journal versions changed. [Maintenance][maintenance]

## 4. Health means several different things

### Layers

| Layer | What it establishes, not what follows automatically |
|---|---|
| Availability | Verified bytes and runtime/provider facilities exist; not device suitability |
| Eligibility | Architecture/KMI, authentication, capacity and policy contract permit an attempt; not successful hooks |
| Load and ownership | Exact loaded/adopted owner and known effects; not policy receipt |
| Delivery | Exact policy generation acknowledged; not behavior |
| Required-feature hooks | Needed runtime hooks installed in the applicable scope; not behavioral or all-process proof |
| Behavioral diagnostics | Scoped proven/leaked/unverified results with controls; not untested APIs, ABIs or old apps |
| Validity and population | Evidence matches current context/consumers; incomplete joins remain incomplete |

Compiled capability is potential, not proof that a candidate is better. A fresh worker does not prove that a two-week-old game adopted new policy.

Use semantic requirements, not hook counts. A requirement identifies behavior and target/profile/API/ABI scope; each backend has its own projection. Kernel and libc hook IDs are not interchangeable. Zygisk libc coverage must not receive credit for raw syscalls. Java, app visibility and Ports failures retain their owners. Never shrink requested features/diagnostic requirements to installed hooks to manufacture green status; equally, an unverified broad product fixture is not automatically a failure of each app's chosen Native features. [Projection][projection], [diagnostic plan][diagnostic-run]

### Classification

| Observation | Default consequence |
|---|---|
| Missing unselected or disabled-optional capability | Information only; keep sufficient protection |
| Selected required runtime hooks missing | Partial coverage; preserve useful safe effects and evaluate bounded recovery |
| Fresh controlled attributable leak in selected scope | Recover the responsible layer; do not assume another backend fixes it |
| Missing fixture, empty RAW stimulus, permission/SELinux/worker error | Infrastructure/unverified; repair observation, not blacklist payload |
| Changed boot/kernel/APEX/policy/image or stale join | Invalidate affected evidence and revalidate; not failure by itself |
| Exact no-effect rejection | Safe candidate traversal may proceed |
| Timeout/crash/ambiguous load | Unknown effect; hold and reconcile |
| No useful effective required protection | Unavailable/full failure; choose a safe recovery plan |
| Some useful coverage remains but requirements are unmet | Partial failure, not “nothing works” |

Decompose `PartialHooks` where trustworthy detail exists. If missing hooks cannot be identified, report unknown coverage, not full protection or zero support. Safety errors remain hard gates. User state distinguishes active-with-scope, partial, checking/unverified, unavailable and held; these proposed display categories do not add a full PASS to the current diagnostic model.

## 5. Selection and bounded recovery

### 5.1 Non-negotiable admission rules

Both modes verify trusted exact bytes and compatible architecture/ABI/contracts, resolve complete policy/capacity, reconcile ownership/old writers, and enforce holds, revocation/preflight uncertainty, quarantine and budgets. Effects must be known absent, known resident or unknown. Only supported retirement or a clean boundary removes conflicting effects.

Manual partial acceptance relaxes coverage sufficiency, never activation safety. A same-owner repair is a supported safe configuration/reconciliation operation, not permission to reload/unload an owner, clear safety holds or fix permanently missing hooks through an unsupported operation.

### 5.2 Auto selection

For identical facts, policy and journal state, the reducer produces the same plan:

1. Reconcile existing work/owner first; an APK open or daemon restart is not a clean slate.
2. Keep a healthy sufficient effective backend. A higher-priority newcomer is a recommendation, not a disruptive promotion.
3. At a clean selection boundary, retain baseline order Builtin, Kmod, authenticated ready KPM, Zygisk. A still-valid prepared recovery target takes precedence only within its bounded episode.
4. Last success may reuse compatible preparation/discovery but cannot replace current eligibility or authority.
5. Missing optional capability needs no recovery; infrastructure/stale evidence needs observability repair.
6. Required gaps or attributable leaks trigger one supported, reasoned same-owner repair, then evaluation of a justified alternative if needed.
7. Core chooses and prepares the next safe action. If a clean boot is needed, stage now and ask only for that action, not “KPM or Zygisk?”.
8. Verify actual results. Incomplete evidence does not cause repeated switches to chase green status.

An untried candidate can be a justified bounded experiment without being a proven improvement. Never load it alongside a live owner just to collect comparison evidence.

### 5.3 Partial coverage comparison

Track semantic sets: requested outcomes, established effective support, fresh behavioral proof, affected required outcomes, candidate potential and known candidate gaps. A reasonable improvement must address an affected outcome without a known loss of another working required outcome or a safety violation. Prefer coverage by set inclusion, not a score.

For incomparable coverage, the proposed default is to retain useful safe current protection and report the gap. Do not silently trade away a required surface or ask an ordinary Auto user to compare implementations. Compiled support nominates a trial; it does not prove its result. Unknown evidence is not automatically a known regression.

Examples: partial kernel route coverage can justify an alternative that closes that gap without losing another requirement. More Zygisk libc hooks do not compensate for losing required raw-syscall coverage. A Java fixture failure does not nominate KPM for a working ko; empty RAW diagnostics nominate no backend.

### 5.4 Full failure and fallback

Without useful protection and with known-clear ownership, Auto may traverse eligible candidates after exact durable no-effect rejection. A resident partial/full owner instead requires planned replacement.

**Zygisk is the principal generic fallback from a failed kernel route**, only when it can address the affected requirements, actual provider/ABI evidence is sufficient, and ownership can safely transfer. It cannot replace every kernel feature or displace a demonstrably more suitable ready candidate merely because it is the fallback.

Ready authenticated KPM can be a planned replacement for Kmod when justified. **Active Kmod → KPM is not same-boot generic fallback:** require retirement or a next-boot plan that prevents old autoload. Provisioning absent KPM infrastructure still needs separate consent.

Builtin persists in the same kernel across reboot. A supported disable/quiescence contract or authorized kernel change is needed before promising another owner; “reboot once and switch” is not generally valid.

### 5.5 Finite recovery episodes

A **recovery episode** is durable work for a specific affected requirement set and material environment/candidate identity. It survives daemon restart, APK reinstall/reconnect, ordinary reboot and receipt compaction. It stores the reason, candidate decisions, evidence, attempted identities, budgets, planned boundary and terminal outcome.

Recommended initial limits, all **proposed policy constants**:

| Budget | Proposed limit | Meaning |
|---|---:|---|
| Same-backend repair | 1 per episode | Only a supported, reasoned repair; an ordinary idempotent reconciliation query is not a new repair |
| Alternative activation attempts | 2 per episode | Each exact candidate at most once; no-effect failures still consume the episode attempt, in addition to existing loader budgets |
| Automatically prepared boot-bound transition | 1 per episode | At most one recovery reboot request for a selected alternative; additional recovery requires a new explicit action or material change |
| Diagnostic infrastructure retry | At most 3 per diagnostic fingerprint | Separate observability budget, with backoff and verified cleanup; does not grant backend trials |

Cold-start traversal and health recovery are distinct: the existing clean-start path can assess the four backend families and attempt each at most once per boot, continuing only on no-effect proof. A health-recovery episode has the two-alternative limit above across boots. If an activation belongs to both contexts, record it once and enforce both budgets; do not nest loops that multiply attempts. The existing per-boot loader budget and payload quarantine remain independently enforced. No episode budget can authorize an attempt that the native journal refuses. Eligibility-only rejections do not spend a real-load attempt; dispatched attempts with unknown results do spend it and enter a hold.

After a trial yields useful safe but partial protection or incomplete diagnostics, retain that result and publish its actual limitations. Do not switch back and forth merely because proof remains incomplete. If it yields a known worse or unavailable result and no safe automatic action remains, prepare a clearly described recovery option such as restoring the last compatible working configuration; do not execute an unbounded rollback/reboot cycle.

An episode can end as recovered, retained-partial, unverified-with-no-further-safe-action, unavailable, blocked, canceled, or exhausted. “Exhausted” stops automatic trials and repeated reboot nagging; it does not suppress the persistent problem in the dashboard.

**Reset rules:** a real change to relevant kernel/payload/provider contract or selected requirements can create a new bounded episode after invalidating affected evidence. A user's explicit “Retry recovery” can authorize a fresh episode but cannot clear unknown effects, quarantine prerequisites, or safety holds. Boot ID alone, timestamp changes, daemon incarnation, noisy diagnostic results, APK opens and Auto/Manual toggles do not reset the recovery budget. Broad diagnostic fingerprints and material recovery fingerprints must therefore be separate concepts.

### 5.6 Manual

Manual exposes `Builtin`, `Kmod`, `Kpm`, or `Zygisk` in advanced settings, with actual availability and limitations. It is not a force-load option.

- The selected family is the only activation target. There is no hidden fallback to another backend.
- A safe partial result can remain effective with named missing required features. Core does not secretly edit the desired policy to make it complete.
- An unavailable, incompatible, unauthenticated or unsafe selection remains unavailable/blocked, with a real next action if one exists.
- Selecting a different family while another remains resident records a **requested** backend and transition plan. It does not relabel the old backend as the new effective one, unload it unsafely, or pretend its effects vanished.
- While waiting for the required boundary, the prior effective protection may remain if safe, visibly labeled as the still-effective backend. The pending request is explicit; this is not hidden fallback.
- Returning to Auto reconciles the actual owner and existing episode. It does not reset budgets, erase holds, or replay a load.

No intermediate “Prefer” mode is proposed initially. It adds ambiguous precedence and recovery semantics without helping the default user.

### 5.7 Decision table

| Current condition | Auto decision | Manual decision | Boundary or proof |
|---|---|---|---|
| Sufficient healthy owner; higher-priority candidate appears | Keep effective; record recommendation | Keep requested/effective owner | No forced reboot |
| Only unselected capability missing | Keep; technical detail only | Same | None |
| Required feature missing; supported in-place repair available | One bounded repair | Repair selected backend only | Exact same-owner contract |
| Required feature missing; justified safe alternative exists | Select and prepare it without asking backend choice | Do not substitute | Clear ownership or planned retirement/boot |
| Useful partial owner; no nonregressive candidate | Retain partial; disclose unmet requirement | Retain selected partial | No speculative trial cycle |
| Confirmed leak in another layer | Recover that layer | Same | Correct attribution |
| Missing/invalid diagnostics | Retry evidence within budget | Same | No backend blacklist |
| Known no-effect load failure | Next eligible candidate within budgets | Stop selected attempt | Durable exact no-effect proof |
| Unknown load or old owner | Hold and reconcile | Same | Never fallback on timeout alone |
| Current Zygisk grant may have been consumed | Keep/recover or stage kernel for next boot | Requested kernel stays pending | Revoke is not retirement |
| New boot; only verified passive Zygisk image, no possible grant consumption | Kernel may be selected first | Honor selected compatible target | Fresh-boot fail-closed grant and clean legacy ownership |
| Active Kmod needs KPM | Planned replacement, not live fallback | Requested KPM pending | Remove old autoload ownership at a safe boundary |
| Builtin remains in same kernel | Use supported same-owner repair or report blocked replacement | Same safety refusal | Reboot alone is insufficient |
| Fallback requires new root/kernel provisioning | Prepare explanation and separate consent plan | Same | Explicit provisioning authority |
| Episode exhausted or candidate remains unverified | Keep safe useful result; stop cycling | Keep selected result | New justified episode/action required |

## 6. Discovery policy and provider limits

### Package and ownership

The ZIP contains one passive pair, `payloads/zygisk/arm64-v8a.so` and `payloads/zygisk/armeabi-v7a.so`. Main `vpnhide_core` has no top-level `zygisk/`, even empty or a dangling link. Core may make an ordinary verified installed copy under its owned projection, provisionally `vpnhide_zygisk_runtime/zygisk/`. Archive deduplication does not forbid that disk copy; no hardlinks, symlinks, moves or always-loaded stub are needed.

The projection has no independent service/activator/policy/updater/download or user ZIP. Manager-visible disable/remove is respected, never silently cleared. This separation avoids inspected Magisk behavior that excludes the *whole* module containing `zygisk/` when global native Zygisk is off, potentially omitting Core service too. [RFC 0003][rfc3], [Magisk filtering][magisk-modules]

### Publication, mapping and hooks

Projection absent on disk (input to a future scan), provider catalog excluding a generation (a proven acceptance/barrier), not mapped, and not authorized to install hooks are four different states. Disk absence does not prevent an existing catalog from offering its retained image. A callback runs after mapping. Unlink does not revoke cached FD/memfd. Grant revocation does not remove installed hooks. A late maps snapshot cannot prove “never mapped this boot.”

**Recommended default:** keep healthy effective protection; retain compatible discovery after successful Zygisk use or when justified by a prepared next-boot fallback; after clean kernel selection, withdraw unneeded discovery at the next natural reboot. Do not demand an immediate second boot for mapping-only cleanup. Passive mapping can consequently persist for two weeks; measure its resource/detection cost rather than calling it free.

For first install or known material Core/kernel/provider change, recommend pre-staging compatible discovery alongside Core when fallback may be needed and a supported manager/provider contract exists. This grants no hooks, does not enable a disabled provider and is not proof of health. A verified kernel-only plan can omit it. Unannounced OTA can still leave discovery absent; late kernel failure may then need one extra boot.

### Fresh boot versus live handoff

After a genuine reboot, verified v2 payloads reject stale grants by boot identity, including before Core/PackageManager is ready. With legacy writers retired and no new production grant that could have been consumed, Core may select kernel even if sticky Zygisk is passively mapped. Cleanup waits for the next natural boot.

In the same boot after possible Zygisk hook consumption, kernel activation remains forbidden under the generic contract. Revocation, missing heartbeat or no targets file does not prove retirement. Old v1 code may ignore the v2 grant protocol and needs separate retirement/migration. Do not weaken the current exposure guard until these proof boundaries are established. [Journal][journal]

### Strict zero-mapping

Strict absence is a stronger alternative, not the proposed default or an initial general UI switch. It needs proven pre-scan/pre-map exclusion across ABIs, unexpected reboot and Core-disabled cases. Without addressed provider support, the system cannot both wait for late kernel selection, guarantee no mapping when kernel succeeds, and activate previously undiscovered Zygisk immediately when it fails. Strict absence accepts an extra fallback boot; unsupported ordering must remain a stated limitation, not a second activator.

### Provider baseline

| Pinned provider | Relevant fact | Supported baseline here |
|---|---|---|
| Magisk `33bfff7c` | Daemon retains boot catalog/FDs; shared ABI companion can map all entries on another module's request. App/Zygote/companion restart does not refresh catalog | Supported full-boot staging; no daemon-restart workaround |
| NeoZygisk `edfa820a` | Ordinary rearm does refresh an existing daemon, but global snapshot replacement, numeric handles and foreign companions are unsafe for addressed changes; exit success is insufficient | No global rearm/private refresh; never `ignore-disable` |
| ZygiskNext public docs `cddf38d5`, wiki `23263b91` | Documents custom-linker Zygote preload; no validated addressed refresh/retirement contract established here | Boot-bound until vendor-supported binary/mode-specific contract is verified |

Sources: [Magisk catalog][magisk-modules], [daemon][magisk-daemon], [companion][magisk-companion], [Neo catalog][neo-daemon], [rearm][neo-rearm], [Next API][next-api], [Next linker][next-linker]. Full SHA pins are in those links; the wiki URL is mutable and the statement refers to recorded snapshot `23263b91088da98420ab32ac95c86c0f2a34083f`. Public Next docs do not pin proprietary binary behavior; custom-linker preload is not a blanket claim about system-linker mode. The [v1.3.3 release](https://github.com/Dr-TSNG/ZygiskNext/releases/tag/v1.3.3) made the custom linker the default, but actual installed mode still needs observation.

Manager and provider are separate support dimensions. Two bundled ABIs do not prove both Zygotes are served: pinned Neo installs a primary ABI branch. Observe actual secondary coverage; reboot does not create it. [Neo installer][neo-installer]

Future hot adapters require addressed immutable identity, boot/provider incarnation, expected generation, idempotency, exact accepted images per ABI, stable generation-bound handles, explicit forward barriers and in-flight FD/retained-consumer accounting. They must preserve foreign modules and disable/remove. CLI status, path digest or one worker ACK is insufficient.

## 7. Durable operation state machine

Effective protection and operation progress are orthogonal: generation A can remain protective while B is staged. A blocked update need not make protection red; successful staging does not make protection green.

Proposed phases, implemented in the existing journal/command engine:

```text
Assessing → Prepared → Staging → AwaitingBoundary
          → ReconcilingBoundary → Activating → Verifying → Completed
```

Same-owner `RepairingCurrent` can lead to verification. `Held`, `AwaitingConsent`, `Unavailable`, `Exhausted` and `Canceled` are explicit states, not completion. Unknown effects suspend the same operation; do not issue an inverse effect blindly.

| Event | Required transition |
|---|---|
| Policy/recovery trigger | Reconcile owner/episode and bind decision to input revision |
| Sufficient current backend | Keep effects; optional recommendation |
| Supported same-owner repair | Journal before execution; record new result without rewriting old receipts |
| Clear-owner alternative | Journal exact attempt before dispatch; verify or hold/no-effect reject |
| Discovery/ownership boundary needed | Prepare complete target; not active |
| Significant provisioning needed | Await specific consent; no patch/flash |
| Manager accepts staging | Verify actual staged identities; await user/natural boundary |
| New boot observed | Verify boot, active image, slot, owner and still-valid plan before admission |
| Wrong generation/disabled manager | Hold/action-required; do not reinstall blindly |
| Exact delivery and scoped evidence | Complete with actual coverage, including partial/unverified |
| Crash/unknown result | Reconcile same operation id; no replay or budget refund |
| Cancel/cleanup | Remove only proven private/staged owned effects; retain live-consumer accounting |

Plans pin operation/episode/config identities, trusted descriptor and rollback floor, active/staged daemon/catalog/payloads, complete ABI projection, manager/provider capability, old owned writers, retained consumers, boundary and cancellation/recovery rules.

Build both ABI files outside scan-visible paths, verify regular files/ELF/hash/root ownership, sync, then use supported manager staging/promotion. No ad hoc active-tree writes, half-generation pair or in-place `.so` overwrite. Separate authoritative catalog and projection recipient identities; arbitrary names/prefixes are not ownership. `module.prop` or timestamps are not running-image proof. [Maintenance][maintenance], [RFC 0003][rfc3]

**Bootstrap proposal:** an explicit manager install invokes a bounded bundled compiled prepare entry using the same Core manifest/policy/journal code, not RPC to an absent daemon. It has no independent service, selector or hook authority and never waits for PM/network/UI in early boot. Certify a real manager contract; otherwise disclose the actual extra activation boundary rather than inventing an activator or claiming one-boot support.

## 8. Proposed configuration and RPC contract

Illustrative canonical extension, not a frozen schema version:

```json
{"backendPolicy":{"mode":"auto","manualBackend":null}}
```

Manual target is exactly `builtin`, `kmod`, `kpm` or `zygisk`; mode/target changes are atomic and expected-revision-bound. Feature requirements stay in canonical protection config. No default UI knobs for weights, retry budgets, provider commands, “Prefer,” untested strict-zero or a second fallback switch. Provisioning consent is a separate plan-bound record, never `allowDangerousOperations`.

One coherent read snapshot exposes:

```text
config_revision, core_epoch, publication_revision
requested_mode/backend, desired_requirements
 effective_backend/generation/scope, layered_health, validity
 recommended_backend/reason, recovery_episode/budgets/operation
 next_boot_intent/required_action
 discovery_by_abi, hook_authority, retained_consumers
 attention_revision/items
```

Unknown/incomplete evidence remains explicit. Per-ABI outcomes distinguish unsupported/not-applicable, prepared, accepted, loaded, authorized, effective, draining and unknown; primary-ABI success is not global success.

Commands express intent: set policy, prepare/approve a permitted plan, cancel where safe, request diagnostics, explicitly retry recovery. All effects are idempotent and revision-bound. No force-activate or generic shell API. Events only announce a new revision; consumers refetch authoritative state. Read-only screen/notification fetches cannot dispatch loads, maintenance or unbounded tests.

RPC/FFI/Kotlin/export must change together. Incompatible Core or unknown required protocol semantics produce explicit compatibility state, not guessed legacy behavior.

## 9. Journal, generations and cleanup

Extend the existing durable owner; no authoritative `lastBackend.json`. `native_boot.json` and exported snapshots remain diagnostic outputs. Keep exposure, holds, accepted package floor, recovery budgets and next-boot work outside receipt pruning. [Journal][journal], [native boot][native-boot]

- Intent precedes effects; a durable valid no-effect result precedes fallback.
- Later leaks/health degradation create new observations/operations, never rewrite confirmed delivery receipts.
- Exact loaded generation matters more than current path; cached FD/memfd images may outlive replaced directories.
- Crash/journal corruption or loss requires conservative reconciliation, not automatic replay from disk guesses.
- Schema migration preserves uncertainty/exposure; lack of a v2 grant says nothing about old v1 hooks.
- Retain generations/packages referenced by active/staged plans, compatible recovery and unresolved consumers. Reclaim only proven owned, unreferenced data under the owner lock.
- Do not clear manual disable/remove, adopt unknown files, change foreign provider/module state or resurrect disabled entries.
- Withdrawal affects future discovery, not already cached/mapped/hooked consumers. Cleanup-only reboot remains optional unless explicitly requested for a separately provable absence property.

## 10. Installation, migration and update lifecycle

Counts below are planning targets under the stated conditions, not device test results or unconditional upper bounds. Count full-device reboots after the operation begins. “Zero extra” means no additional reboot beyond one already required elsewhere. App relaunch and framework loaded-code boundaries are reported separately.

| Scenario | Required preparation and behavior | Reboot expectation |
|---|---|---|
| Fresh manager install, supported kernel route | Stage Core and any justified fallback before first scan; validate actual runtime after boot | Normally one manager-install reboot; do not promise zero without a specific contract |
| Fresh install requiring Zygisk | Stage complete owned projection with Core; verify provider/ABI and new-boot grants | One boot if coordinated staging is supported; missing first discovery can require a second |
| Fresh install with strict unknown-OFF | Do not publish until late kernel failure is known | May require install boot plus activation boot; explicit strict-policy cost, not general default |
| v1 builtin → v2 | Preserve config; retire positively owned old writers; stage Core; adopt compatible builtin after clean boundary | Target one migration reboot. Same kernel still contains builtin; RPC changes do not mandate rebuilding it |
| v1 Kmod → v2 | Do not hot-displace loaded ko; stage replacement and owned autoload retirement together | Target one clean migration boot; uncertain ownership blocks rather than justifies another load |
| v1 KPM → v2 | Account for APatch/manual autoload, credentials and ownership; preserve foreign provisioning | Target one boot when a complete supported plan exists; foreign ownership may need explicit resolution |
| v1 Zygisk → v2 | Stage new projection and retire owned old discovery before the same boot; old hooks remain until boundary | Target one boot, not removal boot followed by a new publication boot |
| Compatible v2 APK-only patch/minor | Reconnect to unchanged compatible Core; preserve config/journal/backend | Zero device reboot for Core/backend |
| APK updates loaded LSPosed Java hook code | Observe actual framework/process loaded-code identity; package install alone may leave old code | A framework/process boundary, often a reboot, may still be needed; do not add a separate unchanged-Core reboot |
| APK and Core RPC are incompatible | Prepare a coordinated compatible target and truthful temporary compatibility state | Usually one planned Core boundary, not separate APK and Core boots |
| Core or backend payload update | Verify and stage complete target, discovery and legacy retirement before user reboot | Target one coordinated boot; zero extra solely for late discovery preparation |
| Core update, selected payload bytes unchanged | Preserve valid compatible payload evidence but verify actual daemon/manager lifecycle | May still need one Core manager boot; unchanged payload is not proof of safe hot daemon replacement |
| Known OTA/kernel change | Pre-stage justified fallback before the already-required reboot when possible | Zero extra if plan and compatibility assumptions hold |
| Unannounced OTA; kernel no longer eligible; no Zygisk catalog entry | Stage fallback after observing new environment | One extra discovery boot can be unavoidable; provider/ABI support still must work |
| Previous boot used Zygisk; new boot has suitable kernel | Before any new production grant, reconcile clean legacy ownership and select kernel | Zero extra for kernel activation; passive cleanup waits for a natural boot |
| Zygisk active in current uptime; better Kmod/KPM appears | Keep sufficient protection; stage a future safe transition | No immediate reboot for mere recommendation; at least the required clean boundary for actual handoff |
| Kernel attempt has exact no-effect proof; Zygisk already accepted | Admit bounded fallback, then establish fresh target/process coverage | Potentially zero additional boot; not applicable to active/unknown kernel effects |
| Active kernel becomes partial/unhealthy | Same-owner repair or planned replacement, not blind loader fallback | Zero for supported repair; replacement may require a boot or separate provisioning |
| Active Kmod → KPM | Stage one owner at next boot; prohibit old Kmod autoload from racing the new owner | Normally one planned boot; not a live replacement promise |
| Builtin → different backend on same kernel | Need explicit supported quiescence/disable contract or authorized kernel change | No generic reboot count; rebooting the same builtin kernel is not removal |
| Zygisk authority revoked; kernel requested in same boot | Keep possible consumption recorded | Current generic contract requires a clean reboot; revocation alone is insufficient |
| Only passive discovery cleanup remains | Stage withdrawal without changing active kernel effects | Zero mandatory reboot now |
| Core crash/restart, same generation | Reconcile journal and exact live identity | Usually zero; ambiguous prior load may require explicit recovery |
| Rollback after failed update | Validate signed compatible target, schema and rollback floor; account for newer cached payload | Often one recovery boot; unsupported downgrade may require external recovery, not journal deletion |
| User disabled Core, projection or provider | Respect disable/remove; do not resurrect it | A reboot alone does not repair an intentional disable; a specific user action is required |

The goal “one migration/update reboot” depends on a complete pre-boundary plan. It does not mean the currently unfinished maintenance executor already provides it. Likewise, a failed manager/provisioning/ABI state is not solved by promising “at most two reboots.”

## 11. User experience and notifications

Lead with effective protection and the actual next action; technical backend choice belongs in advanced Manual/diagnostics. Distinguish “partially protected,” “could not verify,” “recovery prepared; restart to apply,” and “update prepared.” A sufficient current owner plus a better recommendation is not urgent failure. A selected required gap is not merely an optional improvement. Unused backend failure and inert-library cleanup do not produce protection alarms.

The approved bridge is **Core attention reducer → private explicit root/Core wake hint → fresh bounded authenticated RPC snapshot → APK NotificationManager**. The hint contains no authoritative condition or privileged command. Review the actual receiver permission/manifest/root-dispatch admission; neither an exported receiver nor an assumed `exported=false` delivery path is sufficient proof.

Channels are **Updates**, **ProtectionProblems**, **ActionRequired**. Core owns severity, scheduling, coalescing, eligibility and persistent deduplication IDs/revisions. APK renders/cancels and handles Android permission/channel mechanics. Resolved conditions cancel old items; delayed hints refetch rather than resurrect them.

Respect permission/channel denial, force-stop, locked-user limitations, post-unlock/boot grace and platform delivery restrictions. Missed attention remains available on permitted reconnect. No popup, auto-opened activity, forced foreground workaround, repeated wake storm or autonomous reboot.

Move approved update-check scheduling/deduplication from the baseline daily APK WorkManager policy into Core. Reuse supported transport where appropriate; ownership migration does not require a new root Internet stack. Absent/incompatible-Core bootstrap stays narrow and explicit, not a competing ongoing policy. Update availability is not installation consent; explicit update and previously authorized bounded auto-update remain distinct. [Worker][update-worker], [approved queue][ledger]

## 12. Alternatives and why they are not the default

| Alternative | Reason not selected |
|---|---|
| Ask which backend after each partial/failure | Violates Auto's low-cognitive-load contract; Manual already supplies control |
| Always promote to highest priority | Disrupts sufficient long-lived protection and ignores hook retirement |
| Partial means total failure, or partial is always retained without recovery | Both discard necessary distinctions; Auto must preserve usefulness and attempt justified bounded recovery |
| Hook-count/one-score ranking | Kernel/libc/API scopes are semantically different |
| Switch after every diagnostic failure | Infrastructure, stale and non-Native failures are not replacement evidence |
| Always publish forever | Unnecessary mapping surface for kernel-only users; sticky preparation is narrower |
| Strict absence on every uncertain boot | Stronger property but extra fallback boot and ordering requirements; not convenience default |
| Late unlink or tiny always-loaded stub | Cannot undo first mapping or cached images; stub still maps |
| Global rearm/provider/Zygote restart | Shared side effects, missing addressed guarantees; Magisk catalog survives Zygote restart |
| Separate user ZIP/activator | Splits policy ownership and lifecycle |
| Hot Kmod ↔ KPM after empty policy | Logical clear is not unload/quiescence |
| Only `lastBackend` persistence | Loses evidence, pending intent, uncertainty and budgets |
| Two boots for every patch/minor | Unnecessary for APK-only and properly coordinated updates |

The choice is consistent: preserve sufficient protection, recover within bounds, prepare useful discovery and defer mapping-only cleanup—not alternating strict-zero and always-published promises.

## 13. Verification and acceptance

### Host and model tests

Table-test optional versus selected-required gaps, known/unknown partial hooks, semantic/raw-syscall differences, incomparable coverage, Native versus Java/Apps/Ports leaks, empty RAW and stale/infrastructure evidence. Verify stickiness, cold-start order, recovery target, Manual no-fallback, requested/effective divergence and no effects from reads.

Test finite budgets across crash/reboot/pruning; mode toggle, boot ID and diagnostic noise cannot reset them. Check exact no-effect versus unknown results, passive mapping versus grant consumption, fresh-boot kernel admission versus same-boot denial, Kmod→KPM planning and builtin persistence. Validate both ABIs, forged/mixed images, identity/symlink races, stale grants/ACK, disabled owner and unsupported adapters.

### Crash and maintenance tests

Inject failure before/after intent, copy/verify/sync, manager dispatch/readback, ABI publication, old-writer retirement, boot reconciliation, grant, receipt and cleanup. No point permits overlapping owners or turns unknown into safe fallback.

Cover first install, every v1 backend, old mirror layout, APK-only update, changed Core/unchanged payload, compatible/incompatible payload and rollback schema, OTA/slot mismatch, storage exhaustion, corrupt journal, cached old images and cancellation. Assert zero Core/backend reboots for compatible APK-only and no second discovery reboot created by late staging in a known coordinated update.

### Separate device gates

Use separately authorized recoverable lab devices, pinned manager/provider binaries/configuration. Observe actual accepted/retained bytes per ABI, first app/Zygote/system_server/companion/child-zygote mappings, grants and old consumers, secondary ABI and foreign-module continuity. Late maps absence is not never-mapped proof; one 64-bit worker is not all-process proof.

Receiver admission and current-boot grant-boundary evidence are mandatory engineering acceptance criteria, not product choices delegated to users. Require paired protocol/manifest tests and conservative old-state migration; missing callbacks never prove safety. Test long-lived games, unexpected reboot after sticky publication, provider crash/safe mode, disabled Core, and notification denial/force-stop/unlock. Measure passive mapping PSS/private-dirty/startup cost. Research restarts/global commands are not production recovery. Missing, unobservable, unsupported, stale and not-run outcomes never become PASS. Host success is not Android acceptance.

## 14. Phased implementation proposal

After separate approval:

1. Align RFC 0003, RFCs 0001/0002 and ledger; define semantic partial-support contract.
2. Implement pure Core Auto/Manual reducer, consistent partial health and durable bounded episodes; test before adding effects.
3. Complete catalog/recipient split, manager/bootstrap staging/readback, next-boot intent, migration and cleanup under the existing authenticated journal owner.
4. Integrate safe activation/repair/verification while preserving no-effect, quarantine, exposure and ownership guards; consume RFC 0001/0002 evidence honestly.
5. Ship paired RPC/FFI/Kotlin/export, Manual UI, update scheduling and notification bridge; correct obsolete separate-ZIP/manual-KMI help.
6. Run host/crash/ABI/provider/migration/device acceptance and document actual support. Release/device rollout remains separately authorized.

Missing implementation is not a device-only gate; passing host tests cannot certify provider timing or loaded LSPosed code.

## 15. Small remaining decision set

Resolve these once in architecture review, not at each Auto failure:

1. **Sticky and selective first-install/material-change prepublication?** Recommend yes where supported, accepting passive mapping until natural boot; no initial general strict-zero toggle.
2. **Recovery budgets?** Recommend one same-owner repair, two alternative attempts and one automatic boot-bound transition per durable episode; diagnostic retries separate.
3. **Initial manager/bootstrap support matrix?** Certify concrete atomic staging/readback contracts; unsupported combinations stay explicit, not a second activator.
4. **Incomparable semantic coverage?** Retain useful safe current protection and disclose gaps until a reviewed semantic priority exists; no default-user backend comparison.
Auto versus Manual, no hidden Manual fallback, mandatory safety, no autonomous reboot and separate provisioning consent are settled requirements, not open questions.

## Source notes

The source baseline and three independent audit inputs establish code/documentation facts, not runtime acceptance. The audits were `backend-health-selection-policy-audit.md`, `zygisk-lifecycle-reboot-policy-review.md`, and `zygisk-provider-lifecycle-independent-review.md` dated 2 November 2026. The latter's initial strict-zero recommendation was superseded by its lifecycle addendum; this RFC uses its provider facts without importing the old default recommendation. Repository citations below are commit-pinned; the local reports are research inputs, not required runtime artifacts.

[selection]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/core-daemon/src/payloads/selection.rs
[bundled]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/core-daemon/src/native_boot/prepared/bundled.rs#L41-L210
[projection]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/core-daemon/src/payloads/projection.rs
[transition]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/core-daemon/src/native_boot/transition.rs#L201-L280
[native-status]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/core-daemon/src/native_status.rs
[config]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/core-rpc/src/config.rs
[kpm]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/activator/src/kpm.rs#L560-L590
[diagnostic-run]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/diagnostic-model/src/run.rs
[assessment]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/diagnostic-model/src/assessment.rs
[coordinator]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/core-daemon/src/autonomous/coordinator.rs
[kotlin-snapshot]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/diagnostics/AutonomousDiagnosticSnapshot.kt
[journal]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/core-daemon/src/journal.rs#L375-L660
[native-boot]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/crates/core-daemon/src/native_boot.rs
[maintenance]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/docs/notes/core-package-maintenance.md
[ledger]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/docs/notes/unified-core-implementation.md
[rfc3]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/docs/rfcs/0003-conditional-zygisk-publication.md
[update-worker]: https://github.com/okhsunrog/vpnhide/blob/5a14350b988d6a7e37635f324eb517dcf30a838f/lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/BackgroundUpdateChecks.kt
[magisk-modules]: https://github.com/topjohnwu/Magisk/blob/33bfff7ce750c27f157ddb4580e36bf842d6d95b/native/src/core/module.rs#L701-L835
[magisk-daemon]: https://github.com/topjohnwu/Magisk/blob/33bfff7ce750c27f157ddb4580e36bf842d6d95b/native/src/core/zygisk/daemon.rs
[magisk-companion]: https://github.com/topjohnwu/Magisk/blob/33bfff7ce750c27f157ddb4580e36bf842d6d95b/native/src/core/zygisk/entry.cpp#L28-L52
[neo-daemon]: https://github.com/JingMatrix/NeoZygisk/blob/edfa820a79d8f0ab39a29c5c2be0f03a6fc58851/zygiskd/src/zygiskd.rs
[neo-rearm]: https://github.com/JingMatrix/NeoZygisk/blob/edfa820a79d8f0ab39a29c5c2be0f03a6fc58851/loader/src/ptracer/zygote_abi.cpp#L203-L250
[neo-installer]: https://github.com/JingMatrix/NeoZygisk/blob/edfa820a79d8f0ab39a29c5c2be0f03a6fc58851/module/src/customize.sh#L110-L143
[next-api]: https://github.com/Dr-TSNG/ZygiskNext/blob/cddf38d5b1501d0cbf08393767afed09ac478b14/zygisk_next_api.h
[next-linker]: https://github.com/Dr-TSNG/ZygiskNext/wiki/Zygisk-Next-Linker
