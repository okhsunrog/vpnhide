# App state: transition contract

Current 2.0 contract for app observation, diagnostic runs, presentation and
capture integration. Privileged commands and recovery belong to core; see
[core architecture](../core-architecture.md) and
[app intents/drafts](../config-coordinator.md). The app's config execution and
root-mutation transport machines have been removed.

Runtime paths and deadlines are documented in [storage](../storage.md),
[observation coordinator](../observation-coordinator.md) and
[diagnostics](../diagnostics.md).

Scope: process-owned config operations, drafts, observations, diagnostic runs,
measurement applicability, presentation and capture integration. Names below are
domain names, not a commitment to public Kotlin or JSON names, except where a
code identifier is quoted. Wire control-v2/telemetry-v1 remains unchanged.

## 1. Ownership and event ordering

Use related machines, each with one owner, rather than a screen-owned copy of each
state. Each machine is a pure reducer executed by its coordinator:

```text
reduce(previous, event) -> next + effects
```

Reducers do not suspend, read Android services, execute commands or format UI text.
Effects run outside the reducer and return identified completion events.

| Owner | State | Exclusive work |
|---|---|---|
| `ConfigCoordinator` behind `CanonicalConfigRepository` | Initialization, intent queue, confirmed config and core receipts | Prepare and submit one core command; privileged execution/recovery belongs to core |
| Draft registry and lifecycle state holders | Base identity, field patch, draft revision, submitted revision | Short edit/registration transitions; no lock held through I/O |
| `ObservationCoordinator` behind every `StateCache` facade | Last observation, request generation, active request, stale mark, error, quarantine | One physical load per resource; coalesce requests |
| `DiagnosticRunCoordinator` inside `DiagnosticDomain`, behind `DiagnosticsCache` | Request admission, active run, immutable run records, operation impact, the claimed confirmation key | One suite at a time, including export requests |
| Capture (`DebugExport`, `LogcatRecorder`) | Process-owned action, core lease, evidence/errors | Forensic action lifetime and noncancellable release; recording lifecycle belongs to core |
| Pure presentation functions | Eligibility, applicability, evidence summary, Situation, Dashboard assembly | No I/O and no independently mutable verdict |

Deviation from the original design, recorded deliberately: there is no global
event dispatcher and no single presentation revision published per logical
event. The coordinators publish immutable states independently;
`DiagnosticsCache.presentation` is a `combine` of the run view, the routing
observation, the root snapshot, the confirmed config, the operation impact, the
claimed confirmation key and the process inputs, mapped by the pure
`diagnosticPresentation`. Each emission is computed from one instant of all its
sources, which is the property the consumers need; ordering across coordinators
is "the app received it in this order".

`OperationId`, `RunId`, `CaptureId`, `RequestId`, `DraftId` are process-unique.
An effect also carries a step/attempt identity (`EffectTicket`). Accept its result
only if it is the expected outstanding effect. Duplicate events have no effect.
Obsolete observation results cannot publish. Late mutating-command evidence is
routed to recovery, never silently treated as a new operation or allowed to
overwrite a newer config.

An event means the app *received* evidence; coordinator order is not proof of the
order of independent external system changes. Unknown external ordering stays
unknown in the resulting observation.

Dependency direction is fixed: config effects never await dashboard derivation,
diagnostic completion or capture packaging. Diagnostics may await config effects;
capture may await both. Releasing a write lane schedules observation work but does
not await it. Shared helper/staging resources have explicit owners and separate
paths where concurrent use is necessary; do not solve collisions with a global
root lock that reintroduces this dependency cycle.

Universal rules for the transition tables below:

- An invalid user/API request returns a typed rejection and starts no effect.
- An irrelevant, duplicate or obsolete completion leaves state unchanged.
- Every accepted operation/run handle has its own result; do not await the next
  arbitrary terminal value in a global StateFlow.
- Collector/waiter disposal only detaches that consumer. Explicit cancellation is
  a different event and has the phase-specific rules below.
- No operation or run is resumed from an in-memory state after process death.
  Startup reconstructs observable facts. Accepted work has no survival guarantee.

## 2. Initialization and command admission

Config availability distinguishes uninitialized/loading, available, missing and
unavailable/invalid. The shared typed core publication supplies config revision
and startup error; the app never assigns a revision or reads the canonical file
through a second root path. Last-good config is historical display state when
current evidence is unavailable, never a writable base.

A missing canonical file permits explicit bootstrap/import. Corrupt config or
journal never causes an empty-default write. Core exposes read-only diagnostics
and refuses mutation; its resource holds and receipts survive app/process restart.
Accepted command lifetime does not depend on a live app waiter. No app-owned
`vhmutate` process or quiescence probe remains.

The startup self-test intent (`DiagnosticsCache.run`) starts the first suite of
the process and afterwards only joins the active run or reads the latest attempt.
A suite blocked by a condition (VPN off, self excluded, restart pending) is a
terminal `NotStarted` attempt with that eligibility; nothing re-arms an intent
for it. Once the conditions become eligible, the measurement key of that world
is covered by nothing, and the owed confirmation of §7 requests exactly one
automatic suite. All these facts survive Activity recreation and reset on
process death.

## 3. App operation lifecycle and core receipts

The process-owned app actor queues local actions, prepares field write sets and
protects editor drafts. Each dispatch obtains one durable core-issued operation
ID and sends one typed `CoreCommand`; the app does not execute Persist, Secret,
Native or Ports phases. Import with cleanup, credentials and reset use compound
core commands as described in [config coordinator](../config-coordinator.md).

A cancelled waiter detaches. A lost submission response queries the same operation
ID, including when the in-flight publication was missed. Readback failure keeps
uncertainty visible; it cannot establish that no effect ran. Terminal phase
results describe core's receipt. Confirmed persistence remains saved after a
known delivery failure, while Unknown holds the affected resource. Explicit repair
may supersede a hold only where new delivery is confirmed; old receipts remain
immutable. No automatic mutation replay occurs.

The app publishes preparation, dispatch and completion to diagnostic admission in
actor order. Pending intent, observed core receipts and measured app behavior are
distinct. Reads, navigation, drafts and unrelated DataStore settings remain
usable while mutation is held. See section 7 for diagnostic-run consequences.

## 4. Drafts and UI priority

A draft is a base config identity plus normalized field edits with per-field edit
revisions. Key overlap includes ancestors: deleting an app conflicts with any of
its edited fields; changing a role boolean conflicts with editing that role's hook
selection. UI edit publication and draft registration occur in the same event.

| Event | State change / operation result |
|---|---|
| Edit a field | Advance field/draft revision; register patch; immediate UI update |
| Observe a new config | Rebase untouched fields; keep edited fields and their revisions |
| Save | Snapshot submitted patch/revisions into a config operation; disable editing that draft during this save |
| Save persistence confirmed | Clear only fields whose current revision equals the submitted revision; rebase to confirmed config; retain operation's activation progress separately |
| Save fails before persistence | Keep patch; restore editing; report error |
| Save remains unresolved | Keep patch and unresolved operation link; allow local editing after handle resolves unresolved, but no new save until recovery |
| Recovery establishes submitted persistence | Clear only still-matching submitted fields; later edits survive; expose recovery outcome |
| Discard | Remove unsaved patch/registration; does not undo already dispatched persistence |
| Activity recreation | Reattach to same draft and operation IDs; do not unregister |
| Process death | Draft lost by design; next startup reads actual storage |

Editing is enabled again after persistence is confirmed; later edits are a new
draft revision even while the prior activation is still running. The same immediate
switch remains disabled until its operation settles or returns unresolved.

Bridge/UI ordering has two distinct cases:

1. A registered overlapping UI edit exists **before mutating dispatch**: reject
   the whole bridge command with `ui_edit_conflict`, application/field identifiers,
   and `persistence=not_attempted`.
2. A new overlapping UI edit arrives **after mutating dispatch but before bridge
   completion**: retain the draft and flag that bridge operation as superseded.
   Finish/reconcile its effects, then return `ui_edit_conflict` with actual phase
   outcomes (`confirmed`, `failed`, or `unknown`) and the current
   `ui_draft_pending` value (true unless that patch was subsequently discarded).
   Never claim no write occurred, and never auto-save the draft to fake rollback.

A bridge operation completed before an edit cannot retroactively fail. UI priority
means its unsaved values remain displayed/protected and its next save wins on those
fields; it does not mean unsubmitted edits are already installed in the backends.
The bridge error envelope carries structured phase results for this distinction.

Auto-hide reconciliation computes only allowed untouched-field changes and reports
skipped protected fields internally. Bridge mutations are all-or-reject before
dispatch. Whole-config import/reset overlapping any active draft returns a conflict
before dispatch; the UI can save/discard that draft, then explicitly retry. No
silent draft deletion or wildcard replacement over unsaved edits.

## 5. Observation machine

One immutable `ObservationState` contains `lastGood` (with its request and
finish time), `generation`, `active` request, `stale` mark, `error`, `attempted`
and the quarantine fields. `lastGood` carries the generation it was read at. A new
attempt never makes the old value new. Errors are structured and retryable.
`current` is the last good value only while it is not owed a re-read (no active
request, no error, no quarantine, same generation); `value` retains it as display
history regardless.

| Event / guard | Transition and effect |
|---|---|
| Ensure, no observation and no attempt | Start one load |
| Ensure, value/error/load already present | Reuse it; no retry loop from recomposition |
| Explicit equivalent refresh while loading | Join that request's handle |
| Invalidate due to a relevant event | Advance requested generation; retain lastGood; set the stale mark; start load if idle, otherwise request one successor |
| Success for active request at requested generation | Publish observation, clear the stale mark and finish its waiters |
| Success/error for older generation | Do not publish; finish old waiters with `superseded`; start one successor for latest generation |
| Error for current request | Publish Failed, keep lastGood historical; resolve waiters with error |
| Explicit retry after failure | New request identity; loading with previous value/error evidence retained |
| Waiter disappears | Detach waiter; process-owned load continues |

Joining is allowed only when the active request satisfies the caller's minimum
generation and collection-start time. A request for an observation made after a
modal opened or a command settled may need the coalesced successor instead. Its
handle follows that identified request; it cannot accept an older cached terminal
result. A physical-read timeout with unproven helper cleanup uses the resource
quarantine rules in section 7, rather than starting a competing successor.

The old physical read drains before its successor starts; cancellation is not proof
that blocking work stopped. Deadline failure is explicit, not endless loading. A
missing dependency produces `initialization_pending`, not a poisoned cache entry.

Every re-read states its cause. `ReadReason` is `Background` (our own process
invalidated it: a root dependency after a config phase or the startup reconcile,
the foreground-return safety net, the run coordinator's own gate reads),
`Transition` (an external signal that the observed fact may have changed: the
foreground app-VPN poller found a changed fact) or `Explicit` (the user asked).
The `StaleMark(reason, since)` is kept from the moment a re-read is owed until
the current generation publishes or fails; overlapping causes keep the earliest
`since` and the strongest reason, and the request that starts carries it.

Routing is the app-scoped VPN observation (`RoutingGateCache`): one privileged
helper reports this UID's state as `VpnOff`, `SelfExcluded` (excluded), `Routed`
or `Unknown(reason)`, with the framework VPN session identity and interfaces.
The foreground poller samples it silently once a second and requests a shared
refresh only for a changed or recoverable fact; negative states need two equal
samples 750 ms apart so tunnel setup cannot publish a false exclusion. Restart
requirements belong to process readiness, not network facts. The presentation
carries routing as knowledge, not as an admission decision:
`RoutingKnowledge.Known(fact, observedAt)`, `Verifying(lastKnown, reason, since)`
while a re-read is owed or running, or `Unknown(cause, lastKnown)` after a failed
or quarantined read. Derive diagnostic eligibility in this precedence:

1. Initialization incomplete -> `Initializing`.
2. Known restart/reboot requirement -> `RestartApp` / `RestartDevice`.
3. Relevant config effects unsettled/unknown/known failed -> `Applying` /
   `ApplicationUnknown` / `ApplicationFailed` until a later relevant operation
   succeeds or manual recovery resolves it.
4. Routing refresh outstanding or observation uncertain -> `Checking` / `Unknown`.
5. Known no VPN or excluded UID -> `VpnOff` / `SelfExcluded`.
6. Otherwise -> `Eligible`.

One consumer never converts `Unknown` into a positive/negative network fact. All
consumers use this same derivation, including capture guidance and bridge reads.

## 6. Measurement context and applicability

At admission, the Checking observation folds the current observations into the
`MeasurementContext` of the run: the subject (process and boot identity), this
UID's applied configuration (its roles and hook selection plus the global
optional features), the routing identity (the framework VPN session, the VPN
interfaces and this UID's routing verdict), the coverage (the active native
backend, its installed optional hooks and LSPosed liveness this boot, with the
typed layers retained beside the identity string), the `changeEpoch`, and the
root observation ID and time it was read at. Whole-config revision alone is not
this projection. The `MeasurementKey` is the context without its instant:
subject, configuration, routing, coverage and change epoch. Two contexts with the
same key describe the same measurable world.

`changeEpoch` is monotonic and advances at the first mutating dispatch of a
relevant config operation (§7). A known transition never disappears just because
values later match. There is no separate uncertainty epoch: a possible transition
not yet classified is exactly a routing observation that is not `Known`, and it
makes the measurement Unverified until a consistent reobservation restores it.
Undetected external changes remain a limitation, not an asserted guarantee.

| Event | Dependency / run effect |
|---|---|
| UI draft changes only | No applied-context change |
| Relevant mutating effect dispatched | Advance changeEpoch before side effects; interrupt the active run; readiness is Applying |
| Such a write later fails | Reobserve; do not resurrect the interrupted run as current |
| Debug-only or proven unrelated per-app edit, the startup runtime reconcile | No semantic invalidation; a forced activation without a write neither delays nor interrupts a suite |
| Imports, reset, shared UID change, unknown impact | Relevant |
| New routing identity (a re-established tunnel has a new framework network id), new coverage, new process | A new measurement key: the old measurement is Changed |
| Routing re-read in flight, failed or quarantined | Current applicability Unverified; never replace results with empty success |
| Statistics/UI preference change | No self-test invalidation |

"Unrelated app" requires different effective UID and no shared global capability
change. Relevance (`operationAffectsSelfMeasurement`) is decided from the declared
and then the prepared write set: this app's own roles and hook selection
(`apps/<self>/…` or the whole `apps` domain), global optional features
(`settings/optionalFeatures` or the whole `settings` domain), and whole
replacements (import, reset, removal). Relevance only ever increases within an
operation.

An immutable measurement has `RunId`, its start context, the frozen probe plan,
per-probe outcomes, whether it completed or was interrupted, its start and end
times and the end context. Raw observations gathered after a known interruption
are retained as evidence but never attributed as if collected in stable
conditions. Coverage of an old measurement is never recalculated using a new
backend: its report is built against its own retained layers.

Applicability is a pure projection (`measurementApplicability`), not a mutable
flag inside the measurement:

| Inputs, in precedence order | Applicability |
|---|---|
| No measurement | Absent |
| Measurement interrupted, or a known relevant change (epoch) since | Changed |
| No current context, or routing not Known | Unverified |
| Current key differs from the measurement's key | Changed |
| Same key | MatchesLastObservation |

`MatchesLastObservation` is deliberately bounded to observations; neither a TTL nor
an app revision proves continuous backend consumption. A failed observation alone
does not discard a prior measurement: successful reobservation restores
applicability if no known relevant change occurred. Interrupted measurements
cannot be restored this way. Execution/evidence sufficiency is checked separately.

## 7. Diagnostic execution machine

`DiagnosticRunCoordinator` executes `reduceDiagnosticRun` under one short lock
and runs its identified effects (`DiagnosticRunIo`: the context observation and
the phased probes) on the process scope. Keep `active`, optional `pending`, and
the retained immutable records (`lastAttempt`, `lastComplete`) separately. An
unsuccessful new attempt never deletes the last completed measurement; a result
referenced by neither is evicted. Every run is an immutable, identified attempt;
leaving a screen or recreating the Activity detaches a waiter and never cancels
or restarts a run.

States for an admitted request: `Waiting`, `Checking`, `Core`, `Slow`,
`Verifying`, `Draining`. Terminal outcomes (`RunOutcome`):

- `NotStarted` with no probes (blocked eligibility, a failed or unresolved
  operation dependency, a deadline, or explicit rejection).
- `Completed` including leaking or unmeasurable probes.
- `Interrupted` for cancellation or context change, with its evidence retained.
- `Failed` for execution failure, with partial evidence retained.

```mermaid
stateDiagram-v2
    [*] --> Waiting: accepted request with operation dependencies
    [*] --> Checking: accepted request
    Waiting --> Checking: dependencies settled
    Waiting --> Finished: dependency failed or unresolved, cancel or deadline
    Checking --> Core: eligible, resources acquired
    Checking --> Finished: blocked, read failed, cancel or deadline
    Core --> Slow: core probes finished, partial evidence published
    Slow --> Verifying: planned probes finished
    Core --> Draining: error, cancel or context change
    Slow --> Draining: error, cancel or context change
    Verifying --> Finished: classify with end context
    Verifying --> Draining: cancel or context change
    Draining --> Finished: cleanup confirmed or resource quarantined
    Finished --> [*]: resolve this run only
```

| From / event / guard | To and effects |
|---|---|
| No active run / request | Allocate ID and publish Waiting or Checking synchronously before returning the handle |
| Relevant config operation already accepted | Waiting(operation IDs) until those operations settle |
| Checking | Request the context observation: a not-invalidated routing observation is reused, an in-flight read is joined, only a stale, failed or absent one forces a new read |
| Waiting / required operations settle successfully | Checking; fresh context observation |
| Waiting / required operation fails or becomes unresolved | Finished(NotStarted) with that failure |
| Waiting / newly accepted relevant operation | Extend dependency set without extending the original deadline |
| Waiting or Checking / deadline expires | Finished(NotStarted, deadline exceeded) |
| Checking / blocked eligibility | `NotEligible`: Finished(NotStarted) recording the eligibility; no probe; a terminal attempt like any other |
| Checking / observation failed | Finished(Failed, read failed) |
| Checking / eligible | Capture start context and plan; Core |
| Core / core probes finished | Publish the partial evidence of this run; Slow |
| Slow / all planned probes finished | Verifying; observe the end context under the same freshness rule |
| Verifying / same measurement key | Finished(Completed); the measurement becomes `lastComplete` |
| Verifying / key changed | Finished(Interrupted, context changed); evidence retained, `lastComplete` unchanged |
| Core or Slow / a probe is unobservable | Record NotMeasured with its reason; continue the plan |
| Core or Slow / suite execution fails | Draining; stop new probes, await bounded cleanup, then Finished(Failed) |
| Core, Slow or Verifying / relevant change (first mutating dispatch of a relevant operation) | Draining(context changed); retain evidence; do not restart automatically |
| Waiting or Checking / explicit cancel | Finished(Interrupted, cancelled) |
| Core, Slow or Verifying / explicit cancel | Draining(cancelled); no successor until physical probe resources are released |
| Draining / cleanup confirms quiescence | Finished with the recorded reason; admit the pending request |
| Draining / deadline, quiescence not established | Finished with resource uncertainty; quarantine the probe resource; reject requests until the helper returns |
| Any / screen or one waiter closes | No execution transition |
| Finished / late stage response | Ignored; never turns old or incomplete evidence into a new run |

A suite starts in exactly three ways, and nothing else requests one:

1. **Startup intent.** `DiagnosticsCache.run` (`ensure`): the first suite of the
   process; afterwards it joins the active run or reads the latest attempt.
2. **Explicit re-check.** `retryDiagnosticsAndDashboard`, the one entry point of
   "the user asked to check again" (Retry on both screens, pull-to-refresh, the
   post-reset re-check): it refreshes the app-VPN observation, requests one
   explicit run and re-reads the Dashboard's root facts. An explicit request
   always allocates a new run after a completed one; it joins an active explicit
   run, and waits as the single pending successor of an active automatic run, so
   the click cannot be absorbed. No cache refresh requests a run as a side
   effect.
3. **Owed confirmation.** `owedConfirmation` reads the presentation: when this app
   is eligible and the current measurement key is covered by no presented
   measurement, no non-blocked attempt taken under that key and no key the owner
   already claimed, exactly one automatic run is owed for it. The presentation
   carries this as `confirmationPending`; the owner (`confirmMeasurements`) waits
   a 300 ms settle window restarted by every newer presentation, keeps at least
   5 s from its previous request, re-checks the latest presentation, requests
   the run and claims the key on admission. A failed run is answered by Retry,
   never by a loop. Because the rule reads state rather than transitions, a cold
   start already routed, a foreground return after a re-established tunnel and a
   session change seen by the poller take one path, and no edge can be lost to a
   missing baseline. A re-read that reveals no new key reruns nothing (I16).

Request admission while busy:

- An automatic request joins any active non-draining run with the same plan; an
  explicit request joins an active explicit run only. Return the actual shared
  RunId.
- Otherwise allow one pending request; a further one is rejected as busy rather
  than queued without bound. Request identity ignores dependencies: an active
  run already carries every accepted operation.
- A forensic request (`captureId` in its identity) cannot join any run: it waits
  as the pending run for the active one to settle.
- Cancelling an awaiter never cancels a shared run. Only `cancel` on the
  coordinator requests execution cancellation; a joined consumer can only detach.

Operation impacts reach the run through `DiagnosticImpactObserver` and the pure
`reduceDiagnosticImpact`: an accepted relevant operation is added to every new
request's dependencies and sent as `OperationAccepted`; its first mutating
dispatch advances `changeEpoch` and sends `ContextChanged`; its settlement sends
`OperationSettled(id, failure)`. Readiness for eligibility comes from the same
state (an unresolved relevant operation is `ApplicationUnknown`, an in-flight one
`Applying`, a known-failed one `ApplicationFailed`).

Waiting, checking, probing and cleanup all have finite deadlines. Waiting for an
operation that becomes paused finishes NotStarted, not an endless spinner. A
pending request retains its original deadline and is checked again on admission.

Probe resources have a separate lifecycle so a terminal handle never implies that
an uninterruptible helper vanished:

| Resource / event | Next state |
|---|---|
| Free / admitted effect | Busy(effect ticket); only that owner can release it |
| Busy / confirmed completion or cleanup | Free; admit next allowed effect |
| Busy / drain deadline without cleanup proof | Quarantined; resolve waiting requests with resource_unavailable |
| Quarantined / the late helper returns | Free; its late result is discarded; one queued request starts |
| Quarantined / any request | Rejected; the presentation names it (`probeUnavailable`) |

Quarantine affects that resource, not all app reads. External helpers can outlive
the app: startup establishes a safe helper session (versioned staging identity)
before launching probes, even though its in-memory quarantine was lost.
Restarting an Activity cannot clear quarantine.

Every check result carries a stable id: `NATIVE_CHECKS` for the Rust probes,
`NATIVE_EXTRA_CHECKS`, `CORE_JAVA_CHECKS` and `EXTRA_JAVA_CHECKS` for the
Java-implemented ones. The probe plan is derived from these registries at request
time and frozen; per-run outcomes are keyed by id. Ownership in the plan is
structural (Java-implemented native-level probes are unowned); backend-scoped
ownership of the Rust probes is applied by the report from the retained layers.

## 8. Evidence, presentation and Situation

Keep per-vector outcomes, owned/unowned scope and provenance. Native attribution
uses its root differential; Java attribution remains explicitly gate-based
inference. Preserve `NothingToLeak` and permission-blocked outcomes as distinct
from backend suppression.

Freeze the planned vector set at start; never drop unavailable probes from the
denominator to manufacture full coverage. A typed unsupported/not-applicable
reason may exclude a vector only through the probe plan, not a failed read.

For each layer expose counts for backend-hidden, system-blocked, nothing-to-leak,
leaking, not-measured and not-yet-run, plus uncovered observations separately.
Derive the owned-scope conclusion (`EvidenceConclusion`) in this precedence:

| Evidence | Conclusion |
|---|---|
| At least one owned leak observed | `OwnedLeak` |
| No attributable observation of hiding/leak (only nothing-to-leak or unavailable) | `Insufficient` |
| No owned leak, some hiding/blocking evidence, run incomplete or probes unmeasured | `Partial` |
| Complete plan, hiding/blocking evidence, no owned leak | `NoObservedLeak` |

This is not a replacement for module presence/readiness. A system-blocked vector
supports limited visibility, not that a module worked. Unowned leaks stay in a
neutral coverage section and do not inflate actionable issue counts. Findings
from an interrupted run remain observations with context warnings, not a fresh
backend verdict. Nothing here claims every selected app is protected.

**The presentation.** Every consumer renders one projection,
`DiagnosticPresentation`: the shared eligibility; routing as knowledge; the
active run (id, whether it is automatic, stage, partial evidence); the latest
attempt with its outcome, failure, measurement and blocking eligibility; the
latest complete measurement with its results, applicability and evidence
summary; `currentSuccess` (completed execution, sufficient evidence, an
applicable measurement and eligible conditions); `probeUnavailable`; the current
measurement key; and `confirmationPending`. Progress appears immediately and
never uses null as a synonym for loading. A failed latest attempt is visible even
if a previous complete measurement exists. A positive current claim requires
MatchesLastObservation; historical findings remain accessible without it.

**The Situation.** `situation(presentation, now)` classifies once, in one
precedence, into an exhaustive `Situation`: `Initializing`; `Checking(what,
lastKnown, reason, since)` with `what` = the VPN state, the suite or a
configuration change being applied; `VpnOff`; `NotMeasurable` (self excluded);
`ActionNeeded(RestartApp | RestartDevice | ApplicationFailed |
ApplicationUnknown)`; `CouldNotCheck(RoutingUnknown | RunFailed | Interrupted |
ProbeUnavailable)`; `Measured(evidence, staleness)` with staleness `Current`,
`Changed` or `Confirming(reason, since)`. Precedence: process health
(initialization, a quarantined probe) → the action the user owes → a
configuration change applying → routing knowledge → a run in flight → a pending
confirmation → the latest attempt → the measurement. A `Verifying` read is
`Checking` at once for `Explicit` and `Transition` causes and after a 2 s grace
for `Background` ones; inside that grace the last known fact stands and the
measurement is `Confirming`, which every surface renders exactly like `Current`.
An automatic confirmation of a still-applicable measurement is silent for the
length of the run; an explicit re-run, an automatic run after a change, and a
confirmation about to be requested are `Checking(Suite)`, so a stale measurement
is never worded as a result to re-check by hand while the owner is about to
replace it. A run that never started because of a condition is never a failed
check (I13). `DiagnosticsCache.situation` publishes one value per presentation
plus a single re-emission when a Background grace expires while the read is
still in flight; it words, it never schedules a run (I16).

**The surfaces.** The Dashboard hero and the Diagnostics banner are two wording
maps over the Situation (`heroVisual`, `diagnosticScreenDecision`); the tiles,
the issue counts, the results list and the attempt notice stay per-surface side
channels. The hero's colour is a function of the case: `Checking` is neutral with
a progress indicator and a subtitle naming what is checked, keeping the last
known condition's prompt with a busy button; `VpnOff` and `NotMeasurable` are
neutral; `ActionNeeded` and `CouldNotCheck` are Attention with the action or the
cause in the subtitle and the matching prompt; `Measured` takes the worst signal
of the tiles and the dashboard issues, and a changed or insufficient measurement
is at least Attention without ever softening a red one. The top-bar indicator
follows only `Explicit` derivations.

**The Dashboard state** is a projection, not a cache. `DashboardCache` caches
`DashboardRootFacts`, the half derived from the root snapshot alone (modules,
LSPosed, targets, environment, installed optional hooks, the legacy-import
prompt); `DashboardCache.state` combines those facts with the live presentation
and assembles the protection tiles (`protectionVerdict`: the latest complete
measurement rendered against its own retained layers is `Checked`; without one,
a latest attempt blocked by a gated eligibility is `Blocked`; everything else is
`Failed`), the banners and the screen state on every change of either
(`assembleDashboardState`, pure apart from wording). The tiles and the hero
therefore always describe the same instant. The state stays null until the root
facts exist and the suite has a first terminal attempt, so the Dashboard appears
with its first verdict.

**The bridge and the bundle** carry the same projection: `AgentControl.getState`
assembles the Dashboard state from the presentation once it reflects a terminal
attempt (`awaitTerminal`), overlays the current Situation on its tiles
(`overlayCondition`) so its legacy gate says "VPN off" whenever the hero does,
and reports the `diagnostics` summary (eligibility, active run, latest attempt,
measurement identity, applicability, evidence counts and conclusion,
`currentSuccess`, `probeUnavailable`, `lastKnownRouting`, `routingRead`) from the
same value; the legacy `gate`/`report` come from `reportGate` on that value. The
bundle does not carry the Situation itself.

## 9. Capture and API integration

Separate operations: read known state, refresh observations, request/await a check,
export a selected run, perform a fresh check with capture, record third-party logs.
Every export records which operation was requested and which RunId, if any, it
contains. `generatedAt` is payload assembly time; measurement times stay separate.

Implemented: the debug export does not measure on its own. `DiagnosticsCache.captureRun`
submits an explicit request carrying a unique `captureId`, which is part of the
request identity, so a capture can never join a suite whose probes began before
its logging and counter baseline; with a run active it is admitted as the pending
run and waits. Forensic order is: acquire the core debug lease, clear dmesg, then
run. The bundle reports the run's own outcome: only `Completed` may be `ROUTED`;
a blocked eligibility becomes its gate; an interrupted, failed, not-started or
never-admitted run contributes its partial evidence and an explicit reason
instead of a verdict. `exportDebug` returns `Written(file, errors)` or
`Failed(reason)`, so an export cannot be silently lost. Leases belong to captures,
never composables. The process-owned forensic action renews its core lease every
30 seconds and releases it in a noncancellable finalizer. The core journal keeps
the previous user logging intent and pending restoration operation separately
from recorder files. Recorder ack/discard cannot erase restoration. An uncertain
restore remains pending without automatic replay. See the current
[capture and recovery contract](../core-architecture.md).

Cancelling a UI waiter does not cancel the process-owned action or clear an active
recorder before Stop confirms. Package only finalized files or bounded immutable
copies; a recorder whose termination is unproven is never represented as a
completed recording; failure to obtain root evidence never suppresses
already-collected app-side evidence; new measurement/attempt/eligibility meanings
require a bundle schema change and a bridge compatibility decision.

## 10. Invariants

| ID | Must always hold |
|---|---|
| I1 | At most one app mutation has uncompleted or unproven root side effects |
| I2 | Confirmed config is published only from confirmed persistence or identified readback, never from optimistic UI intent |
| I3 | Failed activation never rolls back confirmed config; unknown persistence never becomes a confirmed rollback |
| I4 | No stale observation overwrites a newer confirmed config or clears a newer error/loading state |
| I5 | Every accepted handle resolves at most once to its own operation/run; paused/quarantined outcomes are explicit |
| I6 | No shared work is cancelled because an Activity, collector or joined waiter disappears |
| I7 | An older save completion cannot erase a newer draft field revision |
| I8 | Pre-dispatch bridge conflicts write nothing; post-dispatch conflicts disclose actual effects |
| I9 | No probe suite overlaps another suite or reuses a quarantined probe resource |
| I10 | A completed measurement and its original coverage/context are immutable |
| I11 | Context changes/unknown observations cannot produce a fresh positive summary from old or mixed-context evidence |
| I12 | All-unmeasured or nothing-to-leak-only results cannot claim proven hiding |
| I13 | Gate failure, blocked eligibility, execution failure and detected leak remain distinct in UI/API/bundles |
| I14 | Capture release happens once logically; logging restoration is never falsely acknowledged |
| I15 | No secret enters state, diagnostic payloads, conflict fields or error text |
| I16 | Recomposition, timer ticks and observation refresh do not independently schedule completed-test reruns; a rerun is owed only by a measurement key nothing covers, once per key |
| I17 | Anything that must agree with the presentation (tiles, banners, the bridge's gate) is projected from it, never cached beside it |

## 11. Scenario traces for implementation tests

These are acceptance traces. The pure reducers are tested with explicit fake
effects (`ConfigOperationDataTest`, `ObservationDataTest`, `DraftDataTest`,
`DiagnosticRunDataTest`, `DiagnosticRunCoordinatorTest`, `MeasurementDataTest`,
`DiagnosticEligibilityDataTest`, `SituationDataTest`, `DashboardUiStateTest`,
`DiagnosticConfirmationDataTest`), and `DiagnosticDomainTest` drives the wired
diagnostic domain through its observation flows with a fake helper. Assert
intermediate publications and effect counts, not just final values.

| ID | Events | Required result / invariants |
|---|---|---|
| T1 | Two unrelated config commands submitted from one old screen snapshot | Both patches survive fresh execution reads; I1–I3 |
| T2 | Toggle -> JSON confirmed -> activation fails | Immediate requested position; confirmed position retained with apply error; I2–I3 |
| T3 | Timeout -> initial readback unknown -> automatic repeat unknown | Two read-only checks total; pause; queued handles rejected; no new writes; I1, I5 |
| T4 | T3 -> late transport response without confirmed receipt -> manual readback | Keep uncertainty visible until core evidence resolves it; never duplicate a command; I1 |
| T5 | Root load A -> relevant invalidation -> A success -> load B failure | A cannot publish; B error plus lastGood historical; I4 |
| T6 | UI dirty field -> overlapping bridge command | ui_edit_conflict, no persistence; unrelated field command allowed; I8 |
| T7 | Bridge mutating dispatch -> UI edits same field -> root success | UI patch retained; bridge conflict includes confirmed persistence; I7–I8 |
| T8 | Save revision A -> unresolved -> edit B -> manual readback confirms A | Preserve B; historical handle unchanged; I5, I7 |
| T9 | Active run -> Activity recreated / bridge awaiter disconnects | Same RunId continues; no second suite; I6, I9 |
| T10 | Old NotStarted/Failed -> explicit retry | Synchronously allocate new request; waiter never receives old terminal result; I5 |
| T11 | Core complete -> slow phase error | Partial evidence retained; failed attempt visible alongside last complete measurement; I10, I13 |
| T12 | Complete run A -> VPN off -> VPN on with the same routing identity | A matches again; no rerun. With a new identity (a re-established tunnel): A is Changed, exactly one automatic confirmation, never "re-check needed" meanwhile; I11, I16 |
| T13 | Complete run -> root read error -> consistent successful reobservation, no known change | Unverified then MatchesLastObservation; dates preserved; I10–I11 |
| T14 | Core probes -> relevant save dispatched -> late probe response | Interrupted evidence; late response cannot certify new config; I10–I11 |
| T15 | Eligible -> network event between the two context reads | Interrupted; no fresh differential claim; I11 |
| T16 | Complete suite: all NotMeasured; then nothing-to-leak-only fixture | Insufficient evidence in UI, bridge and export; I12–I13 |
| T17 | Current gate unknown with old successful test | Explicit unknown everywhere; history visible; no endless spinner/current green; I11, I13 |
| T18 | Active normal run -> forensic request | Waits as the pending run; cannot join old probes; I9 |
| T19 | Capture -> user turns debug off -> capture ends twice | Effective logging kept during token; one release; restore current intent; I14 |
| T20 | Capture ends while mutation unresolved | Stop recording; release token; pending cleanup evidence; no barrier bypass; I1, I14 |
| T21 | Blocked self-test / failed root during support collection | Partial artifact when writable, explicit unavailable evidence, no fabricated verdict; I13 |
| T22 | Owned leak plus unowned leak; then only unowned leak | Actionable failure only for owned scope; uncovered observations remain visible; I12–I13 |
| T23 | Cancel running check -> helper cleanup times out -> request new check | Terminal interruption plus resource quarantine; no overlapping helper; I5, I9 |
| T24 | Explicit check waits on save -> save becomes Paused | NotStarted(application_unknown); no indefinite waiter; I5 |
| T25 | Boot changes / app process dies during operation or capture | No claimed continuation; fresh initialization/reconciliation, no inherited draft; I1–I3, I14 |
| T26 | Whole import/reset with open draft; then discard and resubmit | First rejection has no effect; second uses fresh config; I7–I8 |
| T27 | Debug or distinct-UID target edit, or the startup reconcile, with unchanged self projection | No new suite and no false semantic change; I16 |
| T28 | UI render and agent snapshot from same presentation value | Identical eligibility, selected RunId, applicability and findings; I13, I17 |
| T29 | Canonical remember setting persists -> secret command fails or times out | Explicit independent phases/repair; no secret in logs, state or bundle; I1–I3, I15 |
| T30 | Capture reserves suite -> logging command waits on config lane | Config completion does not wait for suite/dashboard; bounded capture outcome, no cycle; I1, I5, I9 |
| T31 | Foreground return while the resume re-read is in flight, then a re-established tunnel | Nothing owed while routing is Verifying; one confirmation for the new key once Known; the same key read again reruns nothing; I16 |
| T32 | A run finishes while the Dashboard state was derived earlier | The tiles follow the presentation on the same emission; no attempt-id bookkeeping; I17 |

## 12. Validation boundary

Pure reducer tests prove decisions for supplied events; they cannot prove the
adapters report reality. Run the Kotlin tests, ktlint, detekt, CPD and Android
lint for code changes, then validate on devices what only devices show: root
timeouts and late effects, real VPN transitions (off/on, split-tunnel membership
changes, a re-established tunnel while the app is backgrounded), Activity
recreation during a run, process restarts and capture cleanup. Device records
are kept in [vpn-poll-device-validation.md](vpn-poll-device-validation.md) and
the pull requests that shipped each stage.

Known gaps, deliberately open:

- No single presentation revision across all coordinators (§1 deviation); the
  diagnostic presentation is one instant of its own sources.

### Shared diagnostic snapshot publication

The executable publication boundary is `DiagnosticSnapshotOwner` inside
`DiagnosticDomain`. Accepted source events and run transitions share the run
owner's lock. The packet contains both presentation and Situation and is
published before terminal handles complete. Config-impact changes and associated
run interruption form one publication. Bridge and export wait for acceptance of
their root checkpoint and pending source events, then consume that same packet.
`DiagnosticSnapshotTest` drives immediate completion waiters, queued source
acceptance, superseded checkpoints, routing/config changes, root invalidation,
waiter cancellation and grace expiry with a virtual clock.
