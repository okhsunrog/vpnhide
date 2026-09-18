# Observation coordinator

Status: connected to the app through `StateCache`, including the shared batched
root reader, Dashboard, targets, statistics, routing readiness and app inventory.
Configuration remains owned by `ConfigCoordinator`; observations never overwrite
its confirmed values or optimistic intent.

## Ownership and publication

`ObservationCoordinator` runs the pure `reduceObservation` reducer under a short
synchronized admission/publication lock. Loads and deadline timers belong to a
process-lived IO scope. Closing a screen or cancelling an awaiting bridge request
only detaches that waiter. Missing application inputs leave a cache pristine;
explicit reads return `InitializationPending` until inputs are supplied.

One immutable `ObservationState` contains the last successful value, active request,
generation, error and quarantine state. Compatibility flows are synchronous
projections of that state. A loader returns its complete payload and must not
publish secondary state. `AppListSnapshot` therefore owns apps, profile names,
partial-scan warning and root observation ID together. The bridge reads this packet
once when rendering an app-list response.

`value` retains the last successful observation while loading and after failure.
It is useful display history, not evidence of current readiness. `current` is null
while loading, stale, failed or quarantined. `RoutingGateCache.gate` uses `current`:
a shared refresh marks the prior value stale before its direct helper read. The
one-second foreground comparison is silent and does not enter this state when its
result is unchanged. A real refresh changes readiness freshness, not diagnostic
result classification or retention.
For eligibility, an invalidated observation that still awaits its re-read is
`Checking`; `Unknown` is reserved for a read that failed, a quarantined source, or
an attempt that never produced a value. Mapping the stale window to `Unknown`
flashed "couldn't determine whether VPN Hide is routed" on every VPN toggle. What
the screens render during that window is not the eligibility but the routing
knowledge the presentation keeps (`RoutingKnowledge.Verifying` with the last
known fact and the read's reason): a user-requested or network-triggered re-read
shows a neutral "Checking…" at once, a background one keeps the last known state
for a 2 s grace (transition contract §8).
`AppVpnStatePoller` runs while the main UI is RESUMED, with a one-second delay
between completed samples. Each sample is a silent root-helper request for the app-scoped
VPN state: current framework VPN session and interfaces plus this app UID's policy
rule membership. It does not read or compare global route/rule text and does not
refresh `RootSnapshotCache`. An equal sample leaves the shared observation untouched,
so the timer cannot make a stable screen enter Checking. Only a changed fact (or a
successful recovery sample after a failed/stale read) requests a process-owned
`StateCache` refresh. That refresh publishes a routed result immediately. `VPN_OFF`
and `EXCLUDED` require two equal samples 750 ms apart; a state/session change continues
settling instead of publishing the tunnel's setup edge as a current negative fact.
Probe failures are ignored by the silent timer, never translated to VPN-off. A
quarantined or active shared read is not replaced by the timer. ON_RESUME always
requests fresh present-tense evidence, and explicit Retry remains available.

The confirmation suite has one owner too, and it is not an edge detector.
`DiagnosticsCache` runs `confirmMeasurements` over its own presentation: whenever
`owedConfirmation` finds this app eligible with a current measurement key
(subject, self configuration, routing identity, coverage, change epoch) that
nothing covers, it requests one automatic run for that key. "Covers" means the
presented measurement was taken under that key, or the latest non-blocked attempt
was (whatever its outcome: a failure is answered by Retry, never by a loop), or
the owner already asked for it. A run in flight or a quarantined probe owes
nothing. Whether a confirmation is pending is itself a field of the presentation
(`confirmationPending`, computed from the key the owner last claimed), so the
Situation words that window as the run it becomes (`Checking`) instead of as a
stale result asking for a manual re-check, and the owner and the surfaces read
one fact. The request waits a 300 ms settle window, restarted by every newer
presentation so the separate emissions of one change produce one run, and keeps
at least five seconds from the previous automatic request so a flapping VPN
cannot keep the suite busy. Because the rule reads state rather than transitions, a cold
start already routed, a foreground return after the tunnel was re-established
(the excluded → included split-tunnel case: a new framework network id is a new
routing identity) and a session change seen by the poller all take the same path,
and none of them can be lost to a missing baseline. A poll that reveals no new key
reruns nothing; the negative confirmation in `RoutingGateCache` still suppresses
the brief false exclusion seen during tunnel setup.

The Dashboard's screen state is a projection, not a cache. `DashboardCache` caches
only `DashboardRootFacts`, the half that costs root shells and changes with the
root snapshot (modules, LSPosed, targets, environment, installed optional hooks);
`DashboardCache.state` combines those facts with the live presentation and
assembles the protection tiles, the banners and the screen state on every change
of either (`assembleDashboardState`, pure apart from wording).

A third source joins that combine: `DeveloperFlagsCache`, the app-local switches
on Settings → For developers that change what the Dashboard says (the agent-bridge
note, the version-mismatch notices). They are preferences, not observations — a
DataStore write invalidates nothing — so holding them inside the cached facts meant
a toggle did nothing until the next refresh. As a source of the projection they
reach the banners immediately and cost no root read. The canonical config's
`debugSwitch` is not one of them: that one genuinely is read off the device and
stays in the environment facts. The tiles and the
hero therefore always describe the same instant: nothing follows the suite, nothing
compares attempt ids, and a fresh measurement can never sit beside the previous
tiles. The state stays null until the root facts exist and the suite has a first
terminal attempt, so the Dashboard appears with its first verdict, as before.

There is no ConnectivityManager listener for VPN-state auto-refresh. The app's own
Java backend intentionally hides VPN lifecycle changes when its visible network
is unchanged, so those callbacks cannot be a reliable trigger. Diagnostic callback
registrations remain: they measure hiding rather than drive readiness refresh.
Device validation on 2026-09-17 is recorded in
[the polling/callback report](notes/vpn-poll-device-validation.md).

The bridge's one-shot state read still joins or reads the terminal attempt
(`DiagnosticsCache.awaitTerminal`) and assembles the same Dashboard state from it
(`loadDashboardState`), so its tiles and the screen's cannot diverge. Refreshing a
cache re-reads that cache's own source and nothing else: `ContextStateCache` has
no hook that starts work in another domain, and `DashboardCache.refresh` never
requests a diagnostic run. The explicit run has one entry point,
`retryDiagnosticsAndDashboard` (Retry on both screens, pull-to-refresh, the
post-reset re-check): it refreshes the app-VPN observation, queues one explicit
diagnostic run and re-reads the Dashboard's root facts. An explicit retry never
joins an already-running automatic suite; it waits as the single pending
successor, so the user's click cannot disappear. The startup intent
(`DiagnosticsCache.run`: the first suite of the process, a join or a read
afterwards), the owed confirmation above and the explicit retry are the only
ways a suite starts. Admission never refuses an automatic request at rest;
whether one is owed is decided from the presentation.
Completed-result retention and measurement classification are unchanged.
Screen retry triggers observe known routing transitions: a temporary unknown value
during refresh does not count as VPN returning. A terminal failure leaves the
Dashboard loading placeholder and remains available for manual retry.

Diagnostic runs are now owned by the process-lived `DiagnosticRunCoordinator`
behind `DiagnosticsCache` (transition contract §7). Its eligibility read at
Checking and its end-context read at Verifying reuse a current app-VPN
observation, join an in-flight read, and force the dedicated helper observation
only when it is stale, failed or absent. The app-VPN observation's session,
interfaces and UID verdict form the measurement routing identity; backend
coverage and boot identity still come from `RootSnapshotCache`. Dashboard
derivation joins the diagnostic run through its own handle and reads its finished
attempt without starting another suite.

Invalidation advances the generation synchronously. A result from an older
generation cannot publish either a value or an error. Its completion starts one
successor for the newest requested generation. Equivalent refreshes join the
active request; an explicit `notBefore` timestamp can demand a newer collection.
Awaiters follow a superseded request to its successor within their own deadline.
An ordinary Ensure does not retry a failed attempt in the same generation.

Every re-read states why it happens. `ReadReason` is `Background` (our own process
invalidated the observation — a root dependency after a config phase or the startup
reconcile, or the foreground-return safety net), `Transition` (an external signal that
the observed fact may have changed: the foreground app-VPN poller found a changed
or recoverable fact) or
`Explicit` (the user asked: Retry, refresh, pull-to-refresh, a manual re-check). An
observation that is owed a re-read carries a `StaleMark(reason, since)`: set by an
invalidation and by a refresh that requests a newer generation, kept while that re-read
is owed or running, carried over to the successor of a superseded read, and cleared when
the current generation publishes or fails. Overlapping causes keep the earliest `since`
and the strongest reason — `Explicit` never degrades to `Background` because a routine
invalidation arrived afterwards. The request that is started carries that reason, so a
read in flight can be attributed after the mark is gone. Nothing renders this yet; the
presentation consumes it in the next stage to word a background top-up differently from
a user-requested re-check.

## Root dependencies

The root reader uses the same coordinator. Each accepted snapshot has an observation
ID and generation. Dashboard, targets and statistics wrap their domain values in
`RootProjection`, retaining this provenance without changing serialized bundle or
bridge models. App-VPN readiness is intentionally independent of this dependency
graph and owns its direct helper observation.

Root invalidation and a new root refresh synchronously invalidate registered
dependents before asynchronous work starts. Registration is lazy and process-lived;
callbacks only invalidate, never read or wait. A cold dependent advances its
generation but does not load before initialization. Repeated invalidations during
an existing read coalesce into one successor, not overlapping root shells.

The mutation coordinator invalidates root observations at every matching phase
completion/recovery before exposing phase evidence. Its separate refresh worker
awaits this shared reload. It never waits for Dashboard/diagnostics before accepting
the next config operation.

Startup self-target preparation runs the same snapshot command as the cache
(including the runtime probe). When it writes nothing, its validated sections seed
the cache's next load whole (`seedSnapshot`), so the cold start does not pay a
second identical root shell; after a config write only the package inventory
seed survives, and any invalidation or explicit refresh discards both seeds. A
seeded value is an observation like any other: it gets the request's observation
ID and generation when consumed.

The startup runtime reconcile (a forced native activation with no config edit) is
one such operation. Cold-start traces on Pixel 8 Pro (2026-09-15, VPN up) showed
its completion invalidating the root snapshot about 430 ms after the first read,
which restarted the routing gate and the targets/Dashboard derivations while the
first derivation and the diagnostic suite were still in flight: a second root
snapshot (about 1.1 s, including package inventory) plus a repeated derivation on
the critical path, `dashboard_ready` at about 3.9 s instead of 2.4 s. The
reconcile is therefore started only after the Dashboard has painted; its
invalidation then refreshes observations in the background.

App inventory subscribes to changes in the accepted `pm_packages` / `pm_users`
sections instead of every root refresh. Config-only or statistics-only changes
therefore do not reload all icons. An explicit app-list refresh also invalidates
its scan. UI and bridge requests share this cache; there is no direct loader bypass.
If the first root read failed, retry also wakes failed dependents, including inventory,
even though no successful source snapshot exists yet.

## Deadlines and physical ownership

Each worker has one observation deadline. Expiration fails its awaiters and marks
the worker quarantined. It does not cancel the worker or start a replacement.
Explicit refresh during quarantine records one retry, but an awaiting caller gets
`ResourceUnavailable`. Only actual worker return releases quarantine; its late
result is discarded. A queued refresh or changed generation then starts one
successor. Without either, the cache remains failed until explicit retry.
There is no automatic endless timeout/retry loop.

The batched root reader uses `RootProcessRunner.runAndDrain`: a command deadline
can stop the launcher, but the worker retains ownership until that process and
its stream readers finish. Output retention is bounded; see [limits](limits.md).
This is not proof that every root descendant exited after closing its pipes.
The snapshot command is read-only and does not intentionally detach work.

Other blocking probes inside the existing Dashboard/routing/diagnostic loaders
still use legacy shell/probe adapters. Their coroutine return is not a new proof
of descendant-process quiescence. Migrating diagnostic execution, cancellation,
capture collection and cleanup is a separate stage.

## Presentation boundary and validation

There is no global atomic swap of every screen. While an expensive projection is
loading, another screen may already show a newer root observation; the former
retains its last-good value. Source IDs make that provenance explicit internally,
and generation rejection prevents the old in-flight computation from publishing
as the new result. A shared presentation revision incorporating measurements and
diagnostic validity remains part of the diagnostic migration.

Host tests control load and timer gates to cover stale success/failure, coalescing,
fresh-since requests, waiter cancellation, missing inputs, retry/quarantine and
source invalidation during a slow projection. Additional tests cover grouped
projections, inventory dependency filtering, config-phase invalidation ordering
and process/pipe draining. A diagnostic-observation integration test rejects the
VPN-off feedback cycle described above. These tests do not establish Android PackageManager,
Binder/root-manager timing or visible behavior on a physical phone.

Validation on 2026-09-15: all 655 JVM tests passed, including 23 new tests, with
no failures or skips. Kotlin warnings-as-errors compilation, ktlint, detekt, CPD
and Android `lintDebug` passed. The source and tests were held unchanged throughout
the final combined Gradle run.
