# App state and synchronization redesign

Status: proposed design; runtime behavior has not changed.

Baseline: `3818954e1615697427664a6bbcff6d9d6483f0c4` (2026-09-15).
Worktree: `../vpnhide_state`, branch `refactor/app-state`, created from the
inspected checkout's committed HEAD. Concurrent uncommitted help work is not
part of this branch. Rebase/integration with that work is a separate step.

This proposal covers the Android app's ownership of configuration, operations,
observations and editor drafts. [Storage](storage.md), [wire protocol](protocol.md)
and [diagnostics](diagnostics.md) remain the current implementation contracts.
The proposal becomes authoritative only as its implementation stages land.

## 1. Problem and historical constraints

Debug logging currently stays in its old position while root persistence,
activation and a root snapshot run. The control has no pending/error state.
Its delay is one symptom of configuration being exposed through an expensive
observational cache instead of a dedicated configuration state.

The history explains the constraints a replacement must preserve:

| Commit | Change | Constraint to preserve |
|---|---|---|
| `1c88be27`, `76938adc`, `c5d96546` | Cache app list, dashboard, targets and checks | Tab switches must reuse completed work |
| `18439129` | Share the root snapshot | Avoid independent duplicate root scans |
| `68eecbbe`, `f7aa683b` | Share cache lifecycle and diagnostic execution | One implementation of loading/error handling; shared check results |
| `b5790766` | Canonical JSON and activators | One persistent configuration; retain package names through incomplete inventory |
| `589bd4cf` | Remove duplicate debug preferences; introduce `debugSwitch` | Separate user intent from temporary capture logging |
| `a29f26be` | Centralize persistence and activation | Failed persistence must not activate an old config |
| `98ed02e1` | Await in-place cache refresh; shared live routing gate | No null flicker; consistent VPN gate across screens |
| `96551621` | Stop rebuilding settings writes through UID projections | Unrelated settings cannot drop configured apps |
| `6b648a44` | Explicit derived-cache list; skip pristine caches | No loading before required dependencies exist |
| `cd5b94e2` | Poll-backed LSPosed debug refresh | Correctness cannot depend on FileObserver delivery |

The July debug change removed the immediate local switch assignment. The August
change retained old values until refresh completed. Together they explain the
current delayed visual response; simply reverting either would restore earlier
consistency problems.

Current structural gaps, established by source inspection:

- `commit(config)` locks after callers have already read and modified a snapshot.
  Two callers can serialize writes derived from the same old config and lose one
  another's changes.
- Its lock spans root I/O and every derived-cache refresh. One exit code conflates
  persistence and activation; an activation error skips refresh even if JSON changed.
- `StateCache.refreshInPlace()` does not register its own job in `inflight`.
  `invalidate()` does not retire an ongoing load. Concurrent callers have no common
  generation check before publishing results or changing loading/error state.
- Screens still call `refreshAfterSave()` after centralized refresh. Capture code
  also refreshes twice. Ownership is duplicated.
- Root refresh does not automatically recompute all its consumers. The shared
  shell reads sections sequentially, so even one root snapshot is an observation
  interval, not an atomic system-wide snapshot.
- Local settings are reactive in Compose, but dashboard messages derived from
  DataStore are cached separately and can remain stale until a dashboard refresh.

## 2. State owners

Extend the existing repositories and caches; introduce testable internal cores
behind their facades. Avoid a parallel set of global stores during migration.

| State | Owner | Lifetime and publication |
|---|---|---|
| UI preferences | Existing `SettingsRepository` | DataStore; reactive `AppSettings` |
| Confirmed canonical config | `CanonicalConfigRepository` | Process state, loaded from and persisted to JSON |
| Accepted config operations and activation outcomes | Same repository/coordinator | Process scope; independent of a screen coroutine |
| Root observations | `RootSnapshotCache` | Immutable observations with freshness and request identity |
| Installed-app inventory | `AppListCache` | Refresh on inventory demand, independently of config changes |
| Dashboard and target presentation | Existing feature facades | Derived from explicit config/observation inputs |
| Live routing precondition | `RoutingGateCache` | Shared probe with VPN/resume/manual triggers |
| Last diagnostic measurement | `DiagnosticsCache` | Results plus measurement context and freshness |
| Unsaved editor changes | Screen state holder | Survive recomposition and activity recreation |
| Active capture tokens | Config coordinator | Process lifetime; released by capture cleanup |
| Hook configuration and statistics | Existing system_server/native owners | Backend-specific; observed by the app |

Compose reads immutable state and sends operations. No composable assigns the
logger flag, writes JSON, or chooses which other screen caches to invalidate.
UI preference changes continue through DataStore; dashboard presentation must
react to those preferences without rerunning root probes.

## 3. Configuration operations

Use typed operations for public entry points, such as `SetDebugSwitch`,
`SetAutoHideSettings`, `ApplyAppEdits`, `ImportConfig`, `StoreSuperkey`,
`BeginCapture`, `EndCapture` and `Reconcile`. Internal pure transformations take
the current config and return a candidate. They perform no I/O and never suspend.

Every producer uses the same coordinator: settings, picker, bridge, startup,
legacy import, auto-hide reconciliation and capture. Full reset must also
exclude concurrent config operations, while retaining its existing prerequisites.

Suggested API shape (illustrative, not a public compatibility commitment):

```kotlin
val operation = repository.submit(ConfigChange.SetDebugSwitch(enabled))
// UI observes config and operation state; bridge callers may await completion.
val result = operation.awaitResult()
```

### Ordered execution

1. Accept the operation with a unique ID in a process-owned scope. Serialize it
   with every other config mutation and explicit activation/reconciliation.
2. Read the current canonical JSON through a dedicated, small root read. Reuse
   the parser and atomic-write builder. Do not load package inventory or the
   whole root snapshot just to read configuration. Distinguish missing, malformed
   and inaccessible config; only explicit initialization may start from empty.
3. Apply the operation to that current config. Validate role/capacity constraints
   with the inventory required by that operation. An incomplete inventory must
   not erase unseen packages or silently broaden roles.
4. Persist the candidate and report the persistence phase independently. Publish
   confirmed config immediately after persistence is established. Derive the app
   logger's effective flag from this publication.
5. Complete coupled work and required activations in order. Record each attempted
   backend's result, including not attempted after a prerequisite failed.
6. Release the operation queue after the side effects have settled. Request the
   relevant observation refresh outside the mutation critical section. Observed
   health is a separate state from command completion.

Initially serialize mutation **through activation**, because existing activators
read the canonical pathname rather than an immutable config supplied by the app.
Letting the next write overtake an older activator would make attribution to a
config version unreliable. Faster switch feedback comes from early state
publication and moving observations out of this queue, not overlapping writes.

Do not silently coalesce accepted mutations in the first version. While a switch
operation is pending, disable repeat input for that control and show its requested
value with a progress indicator. Other changes can enter the ordered queue.

### Result model

Represent phase outcomes in one immutable operation state rather than independent
booleans. Persistence and each activation need separate outcomes:

- Queued / running.
- Persistence confirmed, failed before replacement, or indeterminate.
- Coupled secret operation confirmed, failed, or indeterminate, when applicable.
- Activation command succeeded, failed, indeterminate, or not attempted.
- Observation current, stale, refreshing, or unavailable (separate from mutation).

An atomic file replacement does not make JSON, secret files, LSPosed and native
backends one transaction. Keep successful persistence visible after activation
failure; offer retry of activation against the **current** config. Never roll back
a newer user change by restoring an old whole-file snapshot.

A shell timeout may occur after a write or after a backend accepted a command.
Use explicit phase acknowledgements and bounded readback to establish what is
known. A missing acknowledgement means indeterminate, not automatic failure or
success. `suExec` currently drains output after execution and kills the direct
process on timeout; it is not yet a sufficient transport for streaming phase
acknowledgements or proving descendant processes have stopped. Define and test
that transport before allowing a timed-out mutation's successor to execute.
If completion cannot be established, pause mutations and expose reconciliation.

Secret content never enters observable operation state or command diagnostics.
Superkey changes need a specific phase/recovery plan: canonical preference and
secret persistence can fail independently, and automatic secret rollback is not
assumed. Preserve current backend prerequisite ordering until that plan is tested.

### Cancellation and process boundaries

Cancelling a UI collector or an `awaitResult()` waiter does not cancel an accepted
mutation. The process coordinator finishes bounded I/O and publishes its outcome.
Explicit cancellation is permitted before execution; after side effects begin,
finish or reconcile rather than pretend nothing happened.

This is process ownership, not a guarantee of execution after Android kills the
process. Startup reads actual disk state, restores interrupted capture intent and
reconciles runtime before publishing readiness. No persistent operation journal
is proposed initially.

An in-process revision orders publications; it is not a backend acknowledgement
or an interprocess lock. Fresh reads reduce stale external edits but cannot make
arbitrary root writers transactional. Boot activators and manual root changes
remain external. Do not claim global linearizability or actual per-process hook
application from an activator exit code.

## 4. Observation and refresh semantics

Strengthen `StateCache` behind its current feature facades:

- One immutable load state contains last value, freshness, error and request ID.
- Every load path, including awaited refresh, participates in the same coordination.
- Concurrent equivalent refreshes share work. An invalidation during a load advances
  the requested generation and schedules at most one subsequent load.
- Only the current request may publish a value/error or clear its loading status.
  Cancelling an old job does not by itself prove its blocking work has stopped.
- Retain the last value while refreshing. Failed refresh exposes staleness; an
  initial failure exposes a retryable error. Cancellation is not a load failure.
- Dependencies are supplied at construction/initialization. A VPN callback before
  initialization requests future work instead of poisoning an uninitialized cache.

Root observations carry a request ID, observation start/end times and the config
fingerprint read during the scan. A scan begun before a config mutation cannot
publish old canonical data over the repository's confirmed config. It may retain
historical runtime information only with explicit freshness metadata.

Pure projections receive their input snapshots as arguments. They do not call
`RootSnapshotCache.getOrLoad()` independently. A view combining saved config with
older runtime observations must show pending/unknown application where relevant.
An app revision cannot prove which config every backend has consumed.

Initial refresh policy:

| Trigger | Work |
|---|---|
| UI preference | Recompose presentation; no root scan |
| Debug switch/capture | Save/apply; coalesce runtime observation; no app-icon scan or diagnostic rerun |
| Target/hook/ports edit | Save/apply; refresh runtime; mark affected diagnostic context stale |
| VPN callback/resume | Refresh shared routing gate; reuse/coalesce root work |
| Explicit inventory refresh | Refresh package inventory; reconcile auto-hide against current config |
| Statistics refresh | Refresh counters; no config mutation |

Retain the batched root probe for the first migration. Splitting its shell sections
into separately scheduled probes is a later optimization requiring measurements.
Remove screen-owned post-save resets as each producer migrates; there must be one
refresh owner per operation.

## 5. Debug, drafts and diagnostics

### Debug and capture

The coordinator owns both toggle intent and capture tokens:

```text
effectiveDebug = debugSwitch OR activeCaptureTokens.isNotEmpty()
```

Begin/end are serialized with all other mutations. Token release is idempotent;
overlapping captures do not restore logging until the final token is released.
Every config write preserves/recomputes the effective flag, including imports and
picker saves. Capture starts after its required logging setup finishes, with an
explicit incomplete-capture outcome if a sink cannot be enabled.

Keep existing JSON fields and interrupted-capture startup reconciliation. UI shows
user intent and, when needed, explains temporary capture logging. Zygisk processes
already running retain their specialization-time configuration; successful file
activation cannot be presented as their live logging acknowledgement.

### Editor drafts

Keep a base config identity plus a patch of edited fields, not a stale full config.
Rebase untouched fields onto newly confirmed config. If the same field changed
externally, show a conflict and preserve the draft; do not silently overwrite it.
Imports are explicit replacements and must invalidate/reconcile existing drafts.

Save captures a fixed patch. Either disable editing during that save initially,
or retain later edits as a separate draft revision; successful completion must
never clear edits made after submission. Use a lifecycle state holder for activity
recreation, and retain the existing unsaved-navigation protection. Process-death
draft persistence is a separate decision, not an implied guarantee of `remember`.

### Diagnostics

Keep one shared measurement executor and the separate live routing gate. Attach
measurement context: process/boot identity, relevant self-target configuration,
backend and routing evidence. Debug-only changes do not invalidate a measurement.
Mark potentially affected results stale without automatically rerunning all probes.
Freshness must be visible when a report would otherwise imply current protection.

This changes the current process-sticky result policy and belongs in its own stage
with updates to diagnostics documentation and bundle schema where applicable.
LSPosed's FileObserver plus fingerprint fallback remains independent; an app cache
refresh is not an acknowledgement from system_server. Statistics remain sampled
counters, including the existing debounced LSPosed disk publication.

## 6. Migration and acceptance gates

Each stage must keep the app usable and remove the old path it replaces.

1. **Operation core and transport contract.** Add a testable coordinator behind
   `CanonicalConfigRepository`, injected storage/activation/clock boundaries and
   explicit phase results. Validate timeout/readback and process-scope behavior.
   Introduce a compatibility adapter only if every existing writer still uses the
   same serialization boundary; do not expose full read-modify-write guarantees
   while stale whole-config callers remain.
2. **Migrate all config producers.** Typed changes, capture token ownership, startup,
   bridge and import. Remove public whole-config writes except explicit import or
   initialization. Audit direct activation/reset paths. Preserve bridge function
   names; report partial failure honestly and version any necessary response changes.
3. **Refresh coordination.** Unify request ownership and generations; separate root
   observations from confirmed config. Delete duplicate post-save refresh/reset calls.
4. **Switches and drafts.** Shared pending/error treatment; confirm after persistence;
   activation retry; draft conflicts and lifecycle retention. Keep local DataStore
   settings reactive, including their dashboard messages.
5. **Measurement freshness and profiling.** Add context-aware diagnostic freshness,
   then measure whether splitting the root probe is justified.

Required deterministic tests (fake I/O, explicitly controlled suspension points):

| Scenario | Acceptance |
|---|---|
| Two operations derived from the same initial state | Both unrelated edits survive |
| UI, auto-hide, capture and bridge interleave | One ordering; no stale whole-config overwrite |
| JSON persists; native/ports activation fails | Confirmed config retained; correct phase failure and retry |
| Timeout before/after replacement or activation | Indeterminate state reconciled; no unsafe successor |
| Screen closes or waiter cancels during write | Accepted operation completes; result remains observable |
| Refresh completes out of order | Old result cannot replace new value/error/loading state |
| Invalidation occurs during blocking read | Old result rejected; one follow-up refresh |
| Root refresh fails after successful save | Saved config stays new; runtime marked stale |
| Toggle off during overlapping captures | Effective debug stays on until final release |
| Capture ends twice or process restarts mid-capture | Idempotent release; startup restores user intent |
| Partial inventory or conflicting draft | No dropped unseen roles or lost unsaved edits |
| No-op tab switch/debug-only change | No redundant inventory load/full diagnostic run |

Run applicable Kotlin unit tests, ktlint, detekt, CPD and Android lint for code
stages. The existing repository test checks shell construction; it does not prove
coroutine ordering. Add suitable coroutine test support through a supported
dependency command if required, after verifying the chosen test seam.

Device validation must separately measure tap-to-feedback, persistence, activation
and observation durations for on/off, repeated toggles, VPN events and capture.
Record results with backend/device/build identity; do not infer device performance
from unit tests. No device measurements were taken for this proposal.

## 7. Decisions to close before implementation

- Phase-aware root transport: streaming acknowledgements versus multiple bounded
  commands; establish what happens to root descendants on timeout on supported
  managers. Extra small root calls may be acceptable; measure rather than assume.
- Recovery ordering for Superkey plus JSON changes, including indeterminate writes.
- Exact draft conflict UI and whether draft retention must survive process death.
- Which runtime observations can establish application, versus command success
  only, without changing control-v2/telemetry-v1.
- Diagnostics freshness keys and serialization impact, scoped to stage 5.

## Source map

Paths below are relative to `lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/`:

- Persistence: `CanonicalConfigRepository.kt`, `StorageConfig.kt`,
  `ShellCommandBuilders.kt`, `ShellUtils.kt`, `ConfigChannels.kt`.
- Reads: `StateCache.kt`, `RootSnapshotCache.kt`, `DashboardCache.kt`,
  `DashboardData.kt`, `picker/TargetsCache.kt`, `picker/AppListCache.kt`.
- Lifecycle: `startup/MainActivity.kt`, `startup/StartupCoordinator.kt`.
- Writers: `settings/SettingsScreen.kt`, `settings/FilesystemHidingSettings.kt`,
  `settings/HiddenAppsSettingsScreen.kt`, `picker/AppPickerScreen.kt`,
  `picker/TargetPickerScaffold.kt`, `AgentControl.kt`, `LegacyConfigImport.kt`.
- Debug: `debug/DebugLoggingPrefs.kt`, `debug/DebugCaptureLogging.kt`,
  `debug/LogcatRecorder.kt`, `debug/DebugExport.kt`, `VpnHideLog.kt`.
- Measurements: `diagnostics/DiagnosticsCache.kt`, `diagnostics/RoutingGateCache.kt`,
  `diagnostics/VpnTransportWatcher.kt`, `statistics/StatisticsCache.kt`.
- Other process: `hook/SystemServerConfigCache.kt`, `hook/SystemDataFileWatcher.kt`,
  `hook/HookLog.kt`, `LsposedState.kt`; native activation in `crates/activator/src/`
  at the repository root.
