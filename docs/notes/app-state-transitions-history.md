# App state: implementation history of the transition contract

The stage-by-stage implementation log of the
[transition contract](app-state-transitions.md), split out on 2026-09-17 so the
contract itself reads as the current specification. Sections keep the numbers
they had in the contract (§13–§22); references elsewhere that cite those numbers
mean this file. Each section records what a stage implemented, its exact
boundaries at the time and its validation. Statements about what was "not yet
connected" are historical: the contract says what holds now.

Amendments made to the contract after these stages (the owed confirmation
replacing the transition-driven edge detector, the Dashboard state as a
projection, the single explicit entry point) are folded into the contract's
sections 7 and 8; the original wording of the superseded rules survives here
in §21 and §22 only as the record of how they were reached.

## 13. First implementation: pure transition cores

The first implementation adds ordinary Kotlin data models and top-level pure
functions, without a new dependency, store singleton, coroutine scope, JSON schema
or backend protocol. Effects are values; nothing in these reducers executes root
commands or calls Android services. Existing facades are not connected yet.

Paths below are relative to `lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/`:

| Core | Implemented behavior | Tests in `src/test/kotlin/dev/okhsunrog/vpnhide/` |
|---|---|---|
| `ConfigOperationData.kt`, `ConfigRecoveryData.kt` | Ordered admission/preparation, identified phase effects, early confirmed config, independent failures, bounded readback, held mutations, manual recovery, before/after-dispatch UI conflicts | `ConfigOperationDataTest.kt` |
| `CanonicalSnapshotData.kt`, `StateTransitionData.kt` | Safe effect identities/error categories; detach concrete config and field-path collections at retention boundaries | Config operation snapshot/conflict cases |
| `ObservationData.kt` | Generation-aware load publication, equivalent-request join, successor-generation wait, last-good retention, failure and identified resource recovery | `ObservationDataTest.kt` |
| `DraftData.kt` | Untouched-field rebase, revision-aware save acknowledgement, capture token/user intent versus confirmed logging | `DraftDataTest.kt` |
| `diagnostics/DiagnosticRunData.kt`, `diagnostics/DiagnosticRunControlData.kt` | Identified runs, bounded pending admission, operation dependencies, progressive evidence, context interruption, independent drain deadlines, quarantine, immutable latest attempt/complete measurement | `DiagnosticRunDataTest.kt` |
| `diagnostics/MeasurementData.kt` | Original/start/end context, applicability and scoped evidence summaries; current success also requires eligible conditions | `MeasurementDataTest.kt` |
| `diagnostics/DiagnosticEligibilityData.kt` | Shared readiness precedence; unknown routing cannot reuse historical routed state as eligibility | `DiagnosticEligibilityDataTest.kt` |

The tests supply effect responses explicitly, including obsolete responses and
duplicate deadline events. They do not sleep or depend on actual coroutine
scheduling. A request deadline and the subsequent drain deadline have different
identities; replaying the former cannot expire the latter. Recovery evidence also
belongs to the specific quarantined effect, preventing an old successful cleanup
response from releasing a newer quarantine.

### What these tests establish

- T1–T8: operation sequencing, independent phase results/recovery, field overlap and
  revision behavior. T1 currently proves when preparation happens; typed canonical
  patch construction and real fresh reads remain to be integrated.
- T10–T17: new run identity after old failure, partial evidence retention, context
  change/uncertainty and evidence sufficiency. T17 also exercises the pure shared
  eligibility decision against a failed observation with historical routed data.
- T18/T23/T24: incompatible capture identities cannot join a run, draining resources
  cannot be reused, and failed operation dependencies finish waiting requests.
  Actual logging/baseline preparation for T18 is still capture-adapter work.
- T19: token ownership and desired versus confirmed logging, including duplicate
  release and late confirmation. T29: canonical/secret phase independence; the
  core types never take a secret or arbitrary root-output string.

These are core portions of the traces, not completion of all thirty integration
scenarios. These pure tests do not exercise Android Activity recreation or
coroutine ownership. Likewise, explicit fake responses establish
reducer ordering, not truthful root acknowledgements or hardware behavior.

Validation on 2026-09-15: `./gradlew :app:testDebugUnitTest :app:detekt cpdCheck
:app:ktlintCheck :app:lintDebug` from `lsposed/` passed. The unit suite contains
570 tests, including 44 new transition-core tests; none failed or were skipped.
No APK/device or Android lifecycle validation was performed for this stage.

### Remaining work before runtime connection

1. Startup/config availability and predecessor-quiescence transport. The operation
   core currently starts with an already-readable confirmed config and proven
   predecessor quiescence; its constructor documents this precondition.
2. Typed config intents, their authoritative write/impact sets, validation and
   no-op planning. `Prepare` identifies the operation; the future coordinator
   must associate that ID with its typed intent and read current canonical data
   before constructing the candidate. It must not reuse a screen's full snapshot.
   Whole import/reset and auto-hide draft rules still need their typed producers.
3. The process-owned dispatcher/effect runner: publish before executing effects,
   associate each handle with its own completion, and forward relevant operation
   acceptance/dispatch/completion events to diagnostic dependencies/context.
   Decide absolute adapter deadlines and cancellation authority there. Generic
   observation/draft payloads must be immutable values supplied by their adapters.
4. Connect the existing `StateCache`, config and diagnostic facades; compute real
   self-context projections and probe plans with stable IDs. The pure request
   deadline is armed once on admission, including for pending requests; drain
   effects arm a separate cleanup deadline.
5. Capture reservation/collection/packaging and deferred cleanup through the config
   barrier (T20/T21/T30); UI lifecycle holders and a common presentation revision
   (T9/T28); import/reset integration (T26) and concrete impact classification (T27).
6. Bundle/bridge compatibility, startup/process-death reconstruction (T25), and
   device validation. No serialized types or runtime wire semantics changed in
   this first core implementation.

## 14. Mutation transport implementation

The next stage implements `vhmutate` plus a bounded Kotlin process runner,
versioned binary staging and typed receipt/readback adapters. Its protocol,
quiescence proof, limits, device evidence and exact migration boundary are in
[root mutation transport](../root-mutation-transport.md). The transport is packaged
but existing app writers are not connected yet.

Recovery can consume a still-undispatched sequence in transport metadata. This
fences a root launch delayed beyond the app timeout; it never repeats a config,
secret or activation effect. The “read-only” recovery policy in section 3 excludes
application mutations, while permitting this lifetime-metadata update.

## 15. Coordinator engine and typed config edits

`ConfigCoordinator` now executes the config reducer through identified coroutine
effects; `ConfigRootIo` connects it to the retained root session and receipts.
Typed edits apply to fresh preparation reads, preserve declared intent for conflict
checks, support no-op plans and keep persistence separate from activation failure.
The engine and adapter are exercised by coroutine/adapter tests; existing app
writers and UI are not connected yet. The exact implementation, validation and
all-writer migration boundary are in [config coordinator](../config-coordinator.md).


## 16. Configuration runtime connection

All app configuration producers now use the coordinator and root transport.
Settings switches render optimistic intent with progress; Activity ViewModels
retain editor drafts and acknowledge only saved revisions. Capture logging tokens,
bridge conflicts, cleanup/reset and startup share this ownership. Implementation,
validation boundaries and remaining observation/diagnostic work are tracked in
[config coordinator](../config-coordinator.md). Earlier implementation sections above
record the intermediate stages; their disconnected-runtime statements are historical.

## 17. Observation runtime connection

`StateCache` now executes the observation reducer in a process-owned coordinator.
Root invalidation synchronously advances dependent generations; obsolete success
and failure cannot publish. Awaiters join shared reads and can detach without
cancelling the worker. Read deadlines quarantine a still-running worker until it
returns, with no overlapping replacement. App inventory metadata is published
with its app list, and root-derived projections carry source observation IDs.

This does not implement an atomic presentation revision across all screens and
diagnostic measurements. The existing diagnostic run lifecycle, capture stages
and their legacy subprocesses remain outside this connection. Concrete deadlines,
retry behavior, dependencies and validation are documented in
[observation coordinator](../observation-coordinator.md).

## 18. Diagnostic execution runtime connection

`DiagnosticsCache` is now a facade over a process-owned `DiagnosticRunCoordinator`
that executes `reduceDiagnosticRun` under one short lock and runs its identified
effects on the observation runtime scope. Every suite is an immutable, identified
attempt; leaving a screen or recreating the Activity detaches a waiter and never
cancels or restarts a run (T9). An explicit retry allocates a new run whose handle
resolves only with that run's own result (T10). A slow-phase failure retains the
core evidence beside the previous complete measurement (T11). Explicit cancel
drains the run's outstanding helper jobs; an unproven drain quarantines the probe
resource until the late helper returns, and requests are rejected meanwhile (T23).

The context effect goes through the shared `RoutingGateCache` and folds it with
`diagnosticEligibility` into `DiagnosticContextObservation`. "Fresh" means not
invalidated (`routingReadPlan`): a current observation is reused, an in-flight
read is joined, and only a stale, failed or absent observation forces a new root
snapshot plus routing probe. VPN callbacks and config writes invalidate it, so a
known change always causes a new read, while the end-context read of an
undisturbed run costs no second root shell. A blocked Checking observation is the new
`NotEligible` reducer event: a terminal `NotStarted` attempt that records the
eligibility, launches no probe and does not consume the startup automatic intent.
An eligible observation yields the `MeasurementContext`: process/boot subject, the
self UID's role and hook selection plus global optional features, the observed VPN
interfaces with this UID's routing verdict, and backend/optional-hook/LSPosed
coverage, with the root observation ID. The same observation runs again at
Verifying under the same freshness rule; a changed identity finishes the run as
Interrupted with its evidence retained (T14/T15 for changes visible between the
two reads).

Every check result now carries a stable id: `NATIVE_CHECKS` for the Rust probes,
`NATIVE_EXTRA_CHECKS`, `CORE_JAVA_CHECKS` and `EXTRA_JAVA_CHECKS` for the
Java-implemented ones. The probe plan is derived from these registries at request
time, per-run outcomes are keyed by id, and the report uses the ids instead of
list position or an empty string. The bundle schema is unchanged: the report's
`id` field existed and was empty for Java checks.

Boundaries of this stage:

- Config readiness is treated as settled and `changeEpoch` stays at zero.
  Operation dependencies (Waiting on accepted config operations), epoch
  advancement at mutating dispatch, `Applying`/`ApplicationUnknown` eligibility
  and re-triggering after settlement are the next stage. The startup runtime
  reconcile can therefore still overlap the automatic suite; a self-projection
  change during a run is still detected by the end-context identity.
- Screens, Dashboard, bridge and export keep rendering the legacy
  `DiagnosticsCache.State` projection (`NotRun`/`Running`/`Blocked`/`Failed`/
  `Ready`). An interrupted or deadline-expired attempt renders as `Failed`;
  applicability, evidence sufficiency and the shared presentation revision are
  not surfaced yet. The identified view is available as `DiagnosticsCache.runs`.
- `awaitTerminal` (Dashboard derivation, agent `getState`) joins the active run or
  returns the latest finished attempt; it never retries a terminal Blocked/Failed
  attempt, so a dependent observation invalidated by the eligibility read cannot
  form a cycle. `retry` keeps the existing policy: a completed suite is reused.
- Debug export no longer runs its own independent `runAllChecks`: it requests a
  capture-identified run from the coordinator (§20). The rest of the capture
  machine — reservation, its own states and deadlines — is still not implemented.
- Probe ownership in the frozen plan is structural (Java-implemented native-level
  probes are unowned); backend-scoped ownership is still applied by the report.

Validation on 2026-09-15: 671 JVM tests passed (16 added for this stage: a
coordinator test with gated fake effects and deadlines, a context/projection test
and a reducer test), warnings-as-errors compilation, ktlint, detekt, CPD and
Android `lintDebug` passed. No APK was installed for this stage; Activity
recreation during a real run, root timeouts and the VPN transition between the
two context reads remain device-validation items.

## 19. Operation impacts on diagnostic runs

The config coordinator publishes each accepted operation's lifecycle to a
`ConfigOperationObserver` synchronously from its actor, in dispatch order:
acceptance (before any effect, with the operation's spec), every mutating root
dispatch (phase), the single result delivery, and a later manual recovery.
`DiagnosticsCache` receives it through `DiagnosticImpactObserver` and the pure
`reduceDiagnosticImpact`:

- Relevance (`operationAffectsSelfMeasurement`) is decided from the declared
  write set: this app's own roles and hook selection (`apps/<self>/…` or the
  whole `apps` domain), global optional features (`settings/optionalFeatures` or
  the whole `settings` domain), and whole replacements (import, reset, removal).
  Other apps' roles, debug logging, auto-hide bookkeeping and a forced activation
  without a write (the startup runtime reconcile) are not relevant: they neither
  delay nor interrupt a suite (T27, and the decision recorded in §2 and §6).
- An accepted relevant operation is added to every new request's dependencies
  and sent as `OperationAccepted` to the run reducer, so a waiting or checking
  run waits for it. Request identity ignores dependencies: a request made while
  a run is active joins it (the active run already carries every accepted
  operation) instead of queueing a second suite.
- The first mutating dispatch of a relevant operation advances `changeEpoch` and
  sends `ContextChanged(known)`: a probing run drains and finishes Interrupted,
  a checking run finishes NotStarted(context_changed). Later phases of the same
  operation belong to that change.
- Settlement sends `OperationSettled(id, failure)`: a waiting run proceeds to a
  fresh Checking observation; a failed or unresolved (Paused) operation finishes
  the waiting run NotStarted with that failure (T24).
- Readiness for eligibility comes from the same state: an unresolved relevant
  operation is `ApplicationUnknown`, an in-flight one `Applying`, a known-failed
  one `ApplicationFailed` until a later relevant operation succeeds or manual
  recovery resolves it; `changeEpoch` enters the measurement context.

Boundaries: the epoch is only advanced by config operations; VPN transitions
still enter through the routing gate invalidation and the end-context identity
comparison, and `uncertaintyEpoch` is not implemented. Screens keep rendering the
legacy projection; a run finished NotStarted because of a failed or unresolved
operation renders as `Failed`.

Validation on 2026-09-15: JVM tests (impact reducer and relevance, coordinator
dependency/settlement/interrupt scenarios, observer ordering in the config
coordinator, readiness in the context builder, join ignoring dependencies),
warnings-as-errors compilation, ktlint, detekt, CPD, Android `lintDebug` and the
signed release build. Device validation of a save during a running suite is
pending.

## 20. Capture through the run coordinator

The debug export no longer measures on its own. `DiagnosticsCache.captureRun`
submits an explicit request carrying a unique `captureId`, which is part of the
request identity, so a capture can never join a suite whose probes began before
its logging and counter baseline (§9); with a run active it is admitted as the
pending run and waits. Forensic order is unchanged: acquire the logging token,
clear dmesg, then run.

The bundle now reports the run's own outcome. `debugSelfTestFrom` maps the
terminal attempt onto `gate` / `checkResults` / `selfTestRunId` / `errors`: only
`Completed` may be `ROUTED`; a blocked eligibility becomes its gate; an
interrupted, failed, not-started or never-admitted run contributes its partial
evidence and an explicit reason instead of a verdict. `exportDebug` returns
`Written(file, errors)` or `Failed(reason)`, so an export can no longer be
silently lost — the old path derived the gate with `captureGateFrom`, which
throws when self-routing is unknown and returned `null` for the whole export.
`captureGateFrom` stays: it backs `RoutingGateCache`, the pre-collect warning
and the runs' routing observation.

Not implemented, deliberately: the §9 capture machine itself — Queued /
PreparingLogging / Baseline / CollectingEvidence / ReleasingLogging / Packaging,
its suite reservation, its own deadlines and cancellation path. Capture-logging
acquire/release keeps its current `finally` semantics, and `LogcatRecorder` is
untouched.

## 21. Shared presentation projection

Deviation from §1 and §8, recorded deliberately: there is no global dispatcher
and no single presentation revision published per logical event. The three
coordinators publish immutable states independently; `DiagnosticsCache.presentation`
is a `combine` of the run view, the routing observation, the root snapshot, the
confirmed config and the operation impact, mapped by the pure
`diagnosticPresentation`. Each emission is computed from one instant of all five
sources, which is the property §8 needed; ordering across coordinators remains
"the app received it in this order", as §1 already said.

`DiagnosticPresentation` carries the shared eligibility, the active run (id,
stage, partial evidence), the latest attempt with its evidence, the latest
complete measurement with its evidence, its applicability
(`measurementApplicability`: Absent / Changed / Unverified /
MatchesLastObservation), the evidence summary and `currentSuccess`
(`canPresentCurrentSuccess`). A routing observation that is loading, invalidated,
failed or quarantined makes the measurement Unverified; a consistent reobservation
restores it (T13). A known change (epoch or identity) makes it Changed while the
history stays visible (T12). A failed later attempt is exposed beside the last
complete measurement (T11). All-unmeasured evidence is Insufficient (T16).

The Diagnostics screen renders the projection through the pure
`diagnosticScreenDecision` (classify, then word): current conditions first
(Initializing/Checking → progress, or an existing measurement shown as unverified
while routing is re-read; RestartApp; VpnOff; SelfExcluded; Applying;
ApplicationUnknown; ApplicationFailed; routing Unknown with a retry, never an
endless spinner), then a run in flight (progress, partial evidence listed as
incomplete), then a failed or interrupted latest attempt (its own prompt when no
complete measurement exists, a notice beside the history otherwise), then the
latest complete measurement: Insufficient evidence outranks applicability, and
applicability words the banner (ready, results changed with a retry that starts
a new run, results unverified). The screen no longer overlays the live gate on
the legacy state; the presentation the live gate feeds is what owes the
automatic suite (`owedConfirmation`).

The Dashboard hero renders the projection through the pure `heroDecision`: the
cached tiles are overlaid with the current eligibility (`effectiveProtection`,
which replaces the former live-gate overlay), and a `HeroNote` qualifies the
subtitle. Notes that make a positive claim unsafe (a configuration change
applying, unresolved or failed; unknown routing; a checking state with no
measurement; an interrupted or failed latest attempt; a changed measurement;
insufficient evidence) downgrade a Protected hero to Attention. ResultsUnverified
only names a refresh in flight and keeps the previous status, so a routine
Dashboard refresh does not flicker; this is a deliberate softening of §8's
"positive summary requires MatchesLastObservation" for the duration of one
routing re-read. The skeleton gate uses the projection's active run instead of
the legacy terminal state.

The bridge (`AgentControl.getState`) and the debug bundle carry the same
projection as an additive `diagnostics` object on `VpnHideState`
(`DiagnosticSummaryInfo`: eligibility, active run id and stage, latest attempt
outcome/failure/blocking eligibility, measurement run id, start/end and
observation id, applicability, evidence counts and conclusion, `currentSuccess`),
filled from `DiagnosticsCache.presentation.value` at assembly time. The legacy
`gate`/`report` fields stay with their meaning, so the bundle schema is not
bumped; a `ROUTED` report next to `applicability: Changed` describes an earlier
state, and docs/debug-bundle.md says so. The logcat recorder leaves the object
null. With this, every consumer named in §21 renders the one projection.

Boundary: capture reservation, cancellation and packaging still use the older
orchestration (§20); the §9 capture machine is deliberately not implemented.
`MeasurementContext.coverageLayers` carries the typed layers behind the coverage
identity (backend, installed optional hooks, LSPosed liveness), and the per-check
list and the Dashboard tiles build a retained measurement's report from them
(§6); only an active run's partial evidence is attributed with the live layers.
The derivation is memoised per snapshot observation id, since the presentation
re-derives the context on every emission of any source.
Probe quarantine is now a presentation condition: `DiagnosticPresentation.probeUnavailable`
carries `DiagnosticRunState.quarantined`, the Diagnostics screen words it as its
own banner (outranked by a current condition, outranking the retained history it
keeps listed, because the Re-check button under that history would only be
rejected), the Dashboard hero names it before any eligibility note, and the
bundle/bridge summary carries the same flag. Relevance (§19) is no longer
classified from the submitted write set alone: `ConfigOperationObserver.prepared`
publishes the spec with the prepared write set merged in — the diff of the
transformed candidate against the fresh base — right after the `Prepared` event
is reduced and before its first `Execute`, so a `transform`-only write to
`apps/<self>` delays new runs from that moment and its first mutating dispatch
interrupts an active one. Relevance only ever increases.

The legacy `DiagnosticsCache.State` projection (`NotRun`/`Running`/`Blocked`/
`Failed`/`Ready`) is retired, together with the `DiagnosticsCache.runs` accessor,
the presentation's unrendered `lastAttemptResults` / `staleFailureVisible` and
`StateCache.pristine`, none of which had a consumer. The Dashboard derivation and the bridge still join
the active run or the automatic suite through `awaitTerminal`, which now returns
the shared presentation once it reflects the terminal attempt (attempt ids are
monotonic and runs finish in admission order, so the first presentation whose
latest attempt id reaches the awaited run's id is that instant). The tiles come
from the pure `protectionVerdict`: the latest complete measurement rendered
against its own `coverageLayers` is `Checked`; without one, a latest attempt
that never started because of a gated eligibility is `Blocked`; everything else
is `Failed`. A later attempt that failed or was interrupted therefore leaves the
measured tiles in place and is named by the hero note (T11), where the legacy
projection blanked the tiles. An attempt that never started because of a
condition (`DiagnosticAttempt.blocked`: NotStarted, no failure, an eligibility)
is not a failed check (I13): the hero and the Diagnostics history carry no
notice for it, since the condition is named by the eligibility while it holds
and is stale once it clears; and while a re-check is in flight the hero only
names the confirmation (ResultsUnverified) instead of the attempt it is about to
replace. Found on the device: a Retry with the VPN off, then the VPN coming back,
flipped a protected hero to Attention twice with "the latest check couldn't run". The bridge's legacy `gate`/`report` come from the
same presentation value (`reportGate`: `ROUTED` for a retained complete
measurement, else the blocking eligibility, else null) as its `diagnostics`
summary, so the two no longer describe different instants. The Dashboard's
routed-transition refresh keys on the presentation's latest attempt not being
`Completed`, which is what the legacy `!is Checked` test meant.

## 22. Situation and read reasons

Implemented 2026-09-16 after the design review in
[ui-state-presentation-review.md](ui-state-presentation-review.md), which found
that §21's decomposition (cached tiles overlaid with the current eligibility,
qualified by a note) let the hero claim "VPN hidden" while the VPN state was the
very thing being re-read, and let the hero and the Diagnostics banner keep two
precedence orders that had already drifted apart. This section supersedes §21's
"softening" paragraph and the routed-transition rule above.

Every re-read of an observation states its cause. `ReadReason` is `Background`
(our own process invalidated it: a root dependency after a config phase or the
startup reconcile, the foreground-return safety net, the run coordinator's own
gate reads), `Transition` (the VPN transport or default-network callback said the
fact may have changed) or `Explicit` (the user asked). The observation reducer
keeps a `StaleMark(reason, since)` from the moment a re-read is owed until the
current generation publishes or fails; overlapping causes keep the earliest
`since` and the strongest reason, and the request that starts carries it.

The presentation carries routing as knowledge, not as an admission decision:
`RoutingKnowledge.Known(fact, observedAt)`, `Verifying(lastKnown, reason, since)`
while a re-read is owed or running, or `Unknown(cause, lastKnown)` after a failed
or quarantined read. Its branches mirror `diagnosticEligibility`'s routing branches
exactly, so eligibility keeps its values for the run coordinator and the bundle.
The presentation also names whether the active run is automatic.

`situation(presentation, now)` classifies once, in one precedence, into an
exhaustive `Situation`: `Initializing`; `Checking(what, lastKnown, reason, since)`
with `what` = the VPN state, the suite or a configuration change being applied;
`VpnOff`; `NotMeasurable` (self excluded); `ActionNeeded(RestartApp |
RestartDevice | ApplicationFailed | ApplicationUnknown)`; `CouldNotCheck(
RoutingUnknown | RunFailed | Interrupted | ProbeUnavailable)`; `Measured(evidence,
staleness)` with staleness `Current`, `Changed` or `Confirming(reason, since)`.
Precedence: process health (initialization, a quarantined probe) → the action the
user owes → a configuration change applying → routing knowledge → a run in flight
→ the latest attempt → the measurement. A `Verifying` read is `Checking` at once
for `Explicit` and `Transition` causes and after a 2 s grace for `Background`
ones; inside that grace the last known fact stands and the measurement is
`Confirming`, which every surface renders exactly like `Current`. That grace is
the bounded, reasoned form of §21's softening: a routine top-up by our own process
draws nothing, a user-visible cause or a slow read is named. An automatic
confirmation of a still-applicable measurement is silent for the length of the run
(the run coordinator's deadline bounds it); an explicit re-run is `Checking(Suite)`.
A run that never started because of a condition (`DiagnosticAttempt.blocked`) is
never a failed check (I13). `DiagnosticsCache.situation` publishes one value per
presentation plus a single re-emission when a Background grace expires while the
read is still in flight; it words, it never schedules a run (I16).

The Dashboard hero and the Diagnostics banner are two wording maps over the
Situation (`heroVisual`, `diagnosticScreenDecision`); the tiles, the issue counts,
the results list and the attempt notice stay per-surface side channels. The hero's
colour is a function of the case: `Checking` is neutral grey with a progress
indicator in the icon bubble and a subtitle naming what is checked, keeping the
last known condition's prompt with a busy button; `VpnOff` and `NotMeasurable`
are neutral; `ActionNeeded` and `CouldNotCheck` are Attention with the action or
the cause in the subtitle and the matching prompt; `Measured` takes the worst
signal of the tiles and the dashboard issues, and a changed or insufficient
measurement is at least Attention without ever softening a red one. The bridge
overlays the same condition on its tiles (`overlayCondition`) so its legacy gate
says "VPN off" whenever the hero does. The top-bar indicator follows only
`Explicit` derivations. An explicit re-check always requests a new suite. The
automatic confirmation is no longer an event: `owedConfirmation` reads the
presentation and asks for one automatic run per measurement key that no
measurement, no attempt and no earlier request covers (2026-09-17, replacing the
`Transition`-driven edge detector, which lost the excluded → included edge on a
foreground return because its baseline was invalidated by the resume re-probe).
A gate already routed when the screen composes, a re-established tunnel seen on
return, and a session change seen by the poller therefore take one path. An
own-roles save advances the change epoch and so is a new key, which the owner
confirms once after the operation settles; the foreground-return re-probe stays
`Background`.

Boundary: the activator does not report whether a forced activation changed
anything, so the startup reconcile still invalidates root observations and the
Background grace is what keeps that re-read silent; a targeted invalidation of
runtime-only sections is the cheaper follow-up if the background work matters.
The bundle carries `lastKnownRouting` and `routingRead(reason, pendingMs)` but not
the Situation itself.
