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
a VPN callback marks it stale immediately, before the 750 ms debounced recheck.
This changes readiness freshness, not diagnostic result classification or retention.

Dashboard derivation initializes/joins diagnostics when needed, then observes its
terminal result, including Blocked or Failed, without retrying it. Explicit
Dashboard refresh still requests the existing diagnostic retry policy. This removes
a dependency cycle: diagnostics refreshes the routing/root source, which invalidates
Dashboard; that successor must not start diagnostics again just because VPN is off.
The existing startup, live-routed screen triggers and explicit retry remain. A
config-only background invalidation does not itself retry a terminal diagnostic.
Completed-result retention and measurement classification are unchanged.
Screen retry triggers observe known routing transitions: a temporary unknown value
during refresh does not count as VPN returning. A terminal failure leaves the
Dashboard loading placeholder and remains available for manual retry.

Diagnostic runs are now owned by the process-lived `DiagnosticRunCoordinator`
behind `DiagnosticsCache` (transition contract §18). Its eligibility read at
Checking and its end-context read at Verifying both force a routing-gate refresh,
so one suite reloads the root snapshot twice and invalidates root dependents twice.
Dashboard derivation joins the run through its own handle; the second invalidation
supersedes the in-flight derivation, whose successor then reads the finished
attempt without starting another suite.

Invalidation advances the generation synchronously. A result from an older
generation cannot publish either a value or an error. Its completion starts one
successor for the newest requested generation. Equivalent refreshes join the
active request; an explicit `notBefore` timestamp can demand a newer collection.
Awaiters follow a superseded request to its successor within their own deadline.
An ordinary Ensure does not retry a failed attempt in the same generation.

## Root dependencies

The root reader uses the same coordinator. Each accepted snapshot has an observation
ID and generation. Dashboard, targets and statistics wrap their domain values in
`RootProjection`, retaining this provenance without changing serialized bundle or
bridge models. Routing readiness also registers the shared root dependency.

Root invalidation and a new root refresh synchronously invalidate registered
dependents before asynchronous work starts. Registration is lazy and process-lived;
callbacks only invalidate, never read or wait. A cold dependent advances its
generation but does not load before initialization. Repeated invalidations during
an existing read coalesce into one successor, not overlapping root shells.

The mutation coordinator invalidates root observations at every matching phase
completion/recovery before exposing phase evidence. Its separate refresh worker
awaits this shared reload. It never waits for Dashboard/diagnostics before accepting
the next config operation.

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
