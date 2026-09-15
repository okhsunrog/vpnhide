# Config coordinator implementation

Status: the coordinator engine and its root adapter are implemented and tested.
The production `CanonicalConfigRepository.commit(config)` path still uses its
existing mutex/`suExec` implementation. Screens, startup, capture and the bridge
have **not** been migrated; the running app's switch behavior is unchanged.

This is the execution layer for the [transition contract](app-state-transitions.md),
following the [root transport](root-mutation-transport.md). Keeping it disconnected
until all producers migrate is deliberate: a mixture of tracked and untracked
root writers cannot satisfy the single-mutation guarantee.

## Ownership and effects

`ConfigCoordinator` takes a scope that its production owner must retain for the
process lifetime, a `ConfigCoordinatorIo`, and callbacks for confirmed config and
observation refresh. It owns one event channel, the existing config reducer,
private effect inputs and one completion per accepted operation. Its public flow
is read-only and includes confirmed config, pending toggle intent, active/queued
operation IDs, availability and the latest phase result.

- Event handling publishes state before dispatching the next identified effect.
  Effects run on IO workers outside the actor. The operation lane stays occupied
  across persistence, coupled commands, native/ports activation and reconciliation.
- A cancelled waiter stops awaiting its result; it does not cancel the accepted
  operation or its root process. Initializers join actor-owned waiters, and later
  initialization calls return current availability without resetting a session.
- A fresh canonical read happens when each queued request reaches preparation.
  The actor never accepts a screen snapshot as authoritative confirmed state.
- Confirmed persistence publishes the candidate immediately. Activation failure
  keeps that value and reports the failing phase; unattempted phases remain
  explicit. Observation refresh cannot delay the result or the next operation.
- Refresh uses one conflated worker: overlapping refresh requests request at most
  one subsequent refresh. Its failures cannot kill the mutation actor. This does
  not yet migrate `StateCache` generation handling or establish a common revision
  across all existing screen caches.
- Unknown effects trigger the existing initial readback and one repeat. Continued
  uncertainty delivers an immutable unknown result, rejects queued handles, and
  holds mutation admission. Manual readback can release the hold without changing
  the already-delivered result or replaying remaining phases.

## Fresh edits and conflict boundaries

`CanonicalEditData.kt` defines toggle, settings-set, app-role, explicit app-delete
and whole-config replacement edits. Java plus its hook selection, native plus its
overrides, and ports plus its policy each form one field domain. Payload lists,
sets and nested config collections are detached when constructing an operation.
Private coupled command strings do not appear in its `toString` or public state.

`canonicalEdits(editorBase, desired)` preserves unrelated changes to a fresh
config. Removing an app from an editor snapshot becomes removal of the roles the
editor changed; it does not erase a different role another operation has since
added. A deliberate `RemoveApp` has an explicitly broader write set.

Internal pure transforms can derive additional fields from the fresh config.
Preparation unions their actual diff with the declared intent fields before the
conflict check. Explicit same-value setters keep their intent fields: a bridge
request cannot evade an existing UI draft conflict simply because the current
value already matches its argument. Whole replacement and system reconciliation
also protect registered UI fields. Producers still need their domain-specific
self-target, auto-hide, import and capture policies during migration.

The pre-dispatch guard rejects an overlapping operation as a whole. A draft that
arrives after dispatch is retained; the result reports the conflict together with
actual phase outcomes. Draft registration before initialization and across an
explicit initialization retry is retained. This registry is not an Android draft
ViewModel: Activity retention and revision-aware save acknowledgement remain UI
integration work.

Preparation verifies serialization/readback equality and validates each planned
command's transport framing before persistence can begin. Invalid or oversized
commands yield `ValidationFailed`, not a partial write. No-op operations execute
no root phase; explicit activation repair or a coupled command can run without
rewriting unchanged JSON. Bootstrap is explicit and only accepts a missing
canonical file, never an invalid or unavailable one.

## Root adapter and adoption

`ConfigRootIo` retains one client, session and monotonically advancing command
sequence. Its factory must capture one retained `RootProcessRunner` when wired to
`prepareRootMutationTransport`; replacing the runner during retries would defeat
its bound on outstanding processes/pipe owners.

Initialization distinguishes Open, Missing, Invalid, Unavailable, Paused and
RebootRequired. A newly created same-boot lane returns RebootRequired: it cannot
prove old-version untracked commands have stopped. A verified boot boundary can
open a fresh session. A same-boot Running predecessor stays paused. Neither case
causes a reboot, deletes metadata or launches application config effects.

Execution and recovery are associated with an operation ID and phase as well as
the transport session/sequence. Recovery for another operation or phase cannot
reuse an old receipt. Readback requires matching boot and session. A missing
bootstrap file that remains missing after a proven terminal write is a known
failure; it does not hold the lane indefinitely.

`native_target_cap` is retained as validated scalar evidence by the transport.
The adapter emits its existing safe marker only for the matching native phase,
so the current picker warning parser remains reusable. Raw shell output and
secret command text remain absent from receipts and coordinator results.

## Validation

Validation on 2026-09-15: 607 Kotlin tests passed (26 added in this stage), with
no failures or skips. The Kotlin build, ktlint, detekt, CPD and Android lint passed.
All 27 native Rust tests passed (3 added), as did host clippy and clippy for both
ARM64 and ARMv7 Android targets. Both Android helper binaries were built. The
updated device fixture script passed pinned Ruff checks.

Coroutine tests use explicit channels/deferred gates for root dispatch and
completion rather than timing sleeps. They cover fresh queued edits, joined
initialization, cancelled waiters, optimistic versus confirmed state, persistence
and secret/activation failures, no-op completion, draft conflicts before/after
dispatch, bounded recovery, paused admission and refresh independence. Root-adapter
tests check adoption, predecessor lifetime, session/phase identities, missing-file
readback and capacity warning forwarding.

The native supervisor tests additionally cover bounded output draining and
warning recovery/redaction. Its updated ARM64 executable was exercised through
`adb shell su` on Pixel 8 Pro on 2026-09-15 using the isolated fixture script.
The production canonical file and activators were not touched; no APK was
installed and the device was not rebooted. This does not validate Activity
recreation or the root permission domain of app-originated execution.

## Next integration boundary

Switch production ownership in one coherent migration, not one screen at a time:

1. Retain the coordinator and runner behind `CanonicalConfigRepository`; connect
   startup availability/recheck UI and publish confirmed config independently of
   stale observation caches.
2. Convert every producer: settings, app/hidden-app editors, filesystem settings,
   imports/legacy cleanup, startup self-target/debug reconciliation, automatic
   hiding, capture and bridge activation. Remove full stale-snapshot writes and
   direct activation bypasses. Capture tokens must share admission ordering with
   the debug intent; bridge results must expose actual per-phase outcomes.
3. Add immediate switch/progress presentation and retained UI drafts with their
   original editor base and field revisions. Feed the registry synchronously when
   edits occur, and acknowledge only the submitted revision.
4. Validate the connected paths on a device, including first adoption, interrupted
   root authorization, phase failures, Activity/process recreation and capture
   cleanup. Observation and diagnostic execution migration remains a subsequent
   part of the larger state design.
