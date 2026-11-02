# Observation coordinator

Status: connected to the app through `StateCache`, including the shared batched
root reader, Dashboard, targets, statistics and app inventory.
Confirmed configuration is a read replica of core, delivered through the shared
process `CoreSession`. The app coordinator owns local intent and drafts; observation
caches never become a second canonical writer. See [core architecture](core-architecture.md).

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
while loading, stale, failed or quarantined.

Core owns autonomous diagnostic admission and the retained run/assessment. No
foreign-VPN presence, split-tunnel membership, app-VPN observer, confirmation timer
or local suite executor participates in readiness. Opening a screen, state read,
export or foreground return does not start a diagnostic run. Explicit Retry
coalesces pending process-owned requests; results arrive only by publication.
Real worker and manual forensic callback registrations measure hooks, not readiness.

Dashboard root facts contain inventory, LSPosed, target and environment metadata.
The screen combines them with live Core native/autonomous read models, accepted
config and developer preferences. Source joins qualify root facts; the diagnostic
card remains visible when heavy metadata is unavailable. Native setup/delivery is
not measured hook health. Statistics qualify current marks against full native
source identity and Java boot evidence while forensic counter captures remain raw.

`SettingsStateOwner` retains DataStore state across Activity recreation. No default
preferences substitute for an unknown read. `DeveloperFlagsCache` projects those
preferences immediately without invalidating root data. Canonical debug intent
remains Core-owned. Language changes reword messages without running probes.

Invalidation advances the generation synchronously. A result from an older
generation cannot publish either a value or an error. Its completion starts one
successor for the newest requested generation. Equivalent refreshes join the
active request; an explicit `notBefore` timestamp can demand a newer collection.
Awaiters follow a superseded request to its successor within their own deadline.
An ordinary Ensure does not retry a failed attempt in the same generation.

Every re-read states why it happens. `ReadReason` is `Background` (our own process
invalidated the observation — a root dependency after a config phase or the startup
reconcile, or the foreground-return safety net), `Transition` (an external signal that
the observed fact may have changed: an external source reported changed facts) or
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

Startup self-target preparation joins RootSnapshotCache's process-owned read.
It never dispatches a competing RootSnapshot RPC. No whole-snapshot or package
inventory seed is retained: after a real config write, the configuration owner
invalidates and refreshes the shared observation normally. Explicit startup Retry
marks that observation stale only when creating a new preparation task, and all
waiters join it. Delayed initial Open and successful reconnect do not repeat a
successful preparation. A genuine Core reconnect refreshes a root observation
that may have failed offline, even if the canonical/backend facts are unchanged.
Once the current process requires restart, a later preparation cannot clear it.
Loader failures retain their generation-bound cause; unavailable, protocol-skew,
truncated and incomplete replies never become successful preparation.

Core owns boot-time Native/Ports delivery. Opening the APK does not force another
`RepairActivation` after first paint: that redundant operation invalidated root
facts and repeated an already-completed diagnostic suite. Explicit repair and real
configuration edits still invalidate observations through their normal phase
receipts. The first Core publication only initializes confirmed state; startup owns
the first root snapshot, so an unsolicited inventory refresh cannot compete for
Core's single reserved root-probe slot.

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

Heavy root reads are fixed core diagnostics on the shared session, with bounded
child execution and output. They retain process observation deadlines and generation
rejection. Current config, debug lease, native decision and app-VPN readiness come
from core publications, independently of heavy forensic snapshots. App-context
probes keep their own physical-worker/quarantine rules; a disconnected waiter does
not prove a probe stopped. The 1.x process/shell-draining validation below is history,
not evidence for the new root transport.

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

## Diagnostic publication boundary

`AutonomousDiagnosticSnapshotOwner` publishes one immutable accepted packet of
Core/config/root source identities and retained run/assessment evidence. Config
joining uses canonical contents, epoch and desired revision rather than a global
publication cursor, so unrelated state publications do not withdraw an unchanged
configuration. Root metadata has a separate join against the fields it consumed.

Bridge/export `awaitSnapshot` waits for a requested root checkpoint or a newer one
and returns an existing owner publication. It does not reconstruct a context from
independent flow reads or admit a measurement. Disconnect releases waiters with
retained evidence explicitly unavailable. This is coherent accepted knowledge,
not simultaneous physical sampling. See [diagnostics](diagnostics.md).

The root observation budget is 225 seconds, beyond the 210-second RPC and 200-second
collector. Root-dependent projections add this source budget before their own
allowance. Forensic captures cannot consume the standalone RootSnapshot slot.
State and receipts remain readable while PackageManager/metadata collection waits.
Virtual-time regressions cover valid slow collections, deadline quarantine and
rejection of late results. Final startup acceptance is recorded separately.
