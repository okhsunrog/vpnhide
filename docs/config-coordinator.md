# Config coordinator implementation

Status (2.0): connected to the running app. `CanonicalConfigRepository` owns one
process-lived coordinator whose only effect adapter is `CoreConfigIo`: every write
is an operation submitted to VPN Hide Core over its socket. Settings, both app
editors, startup, legacy cleanup, import/reset, capture logging and bridge
mutations use this lane. The app has no root runner, no `su` and no shell.

This implements the configuration portion of the [transition contract](notes/app-state-transitions.md)
on top of core's journaled operations ([core-2.0 architecture](notes/core-2.0-architecture.md)). Observation generations
are now connected through the [observation coordinator](observation-coordinator.md).
Diagnostic runs and capture reservation/packaging remain separate migration work.

## Ownership and effects

`ConfigCoordinator` takes a scope that its production owner must retain for the
process lifetime, a `ConfigCoordinatorIo`, and callbacks for confirmed config and
observation refresh. It owns one event channel, the existing config reducer,
private effect inputs and one completion per accepted operation. Its public flow
is read-only and includes confirmed config, pending toggle intent, active/queued
operation IDs, availability, active logging tokens and phase results. A separately
retained recovery result lets an editor acknowledge its own earlier unknown save
even when a later operation has already replaced the latest result.

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
- Each matching phase completion/recovery synchronously invalidates the root
  observation before publishing phase evidence or resolving callers. Registered
  dependents advance their own generations immediately. A conflated worker awaits
  the shared root reload independently of mutations; it no longer loops through
  and refreshes each derived cache. Its failures cannot kill the mutation actor.
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
also protect registered UI fields. Self-target and auto-hide policies run on the
fresh prepared config. Imports explicitly replace all domains; manual hidden-app
selection declares only its hidden fields, allowing unrelated role drafts.

The pre-dispatch guard rejects an overlapping operation as a whole. A draft that
arrives after dispatch is retained; the result reports the conflict together with
actual phase outcomes. Draft registration before initialization and across an
explicit initialization retry is retained. Activity-owned `CanonicalEditorViewModel`
instances retain field drafts through recreation and overlays without persisting
them across process death. Confirmation removes only submitted revisions. A new
edit made while saving, including a return to the old confirmed value, survives
the old acknowledgement. Discard advances the revision to prevent accidental reuse.

Preparation verifies serialization/readback equality, checks the candidate
against core's config size limit (`MAX_CORE_CONFIG_BYTES`, 512 KiB) and validates
each coupled root effect before persistence can begin. Invalid or oversized
candidates yield `ValidationFailed`, not a partial write. No-op operations execute
no root phase; explicit activation repair or a coupled command can run without
rewriting unchanged JSON. Bootstrap is explicit and only accepts a missing
canonical file, never an invalid or unavailable one.

## Core adapter

`CoreConfigIo` (`lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/CoreConfigIo.kt`)
maps coordinator phases onto core:

- **Initialize / read** take core's published state (`state_since`): the config,
  its revision and the operations in flight. An operation still running in core
  (for example the boot apply) starts the lane Paused.
- **Persist** submits `Replace(candidate)` against the revision of the fresh read
  (`config_replace`). Core persists, then applies Native and Ports inside the same
  journaled operation; the receipt carries one outcome per phase.
- **Native / Ports** report that operation's outcomes. With no current core
  operation (a forced reconcile, or after a coupled command) they submit
  `RepairActivation` (`config_repair`) so core re-applies from its durable config.
- **Secret / Cleanup** are fixed core root effects (`root_effect`: set/clear the
  APatch SuperKey, delete pre-1.0 legacy files, full reset). They are idempotent,
  so an uncertain one is recovered by applying the same effects again.
- **Recovery** asks core for the same operation id (`get_operation`); an uncertain
  submission is never resubmitted as a new edit. A refused edit, an unreachable
  core or a protocol skew is a known failure with nothing written.

Core's operation journal (`/data/adb/vpnhide/core/operations.json`) replaces the
1.x helper's session, lock and receipts, so there is no first-adoption step.

Known gap: the receipt has no native target-capacity field, so the picker's
`native_target_cap` warning (`ConfigPhaseEvidence.nativeCapacity`) is never set in
2.0; core only logs the overflow (see [limits.md](limits.md)).

## Validation (1.x transport, historical)

The validation notes below were recorded for the 1.x root-transport adapter and are kept as history.

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

## Connected UI and lifecycle

Canonical switches display requested intent immediately and remain busy through
their operation. Known persistence failure returns to confirmed state. Persistence
success followed by activation failure keeps the saved value and offers explicit
activation repair. Secret and cleanup failures direct the user to the original
action; backend activation cannot repair them. Initial/automatic/manual readback
has visible progress, and a continued unknown outcome pauses config writes.
Agent control remains an ordinary DataStore preference and does not use root.

Configuration feedback has one UI owner across screen navigation. Routine saves
show progress inside the switch thumb without inserting status rows or changing
the control width. Success is silent. Known failures use an overlay Snackbar;
confirmed persistence followed by activation failure offers explicit activation
repair. Invalid/unavailable state and uncertainty
remaining after automatic readback use a dismissible dialog. Dismissal does not
reopen it on ordinary recomposition/navigation; an attempted configuration change
opens it again without submitting the mutation. Manual rechecking shows progress
inside the dialog. Browsing retained app lists remains available while writes are
paused. Neither notifications nor recovery add a permanent settings entry.

The feedback update passes 632 JVM tests, warnings-as-errors compilation, ktlint,
detekt, CPD and Android lint. Tests cover dialog acknowledgement/reopening, quiet
automatic readback, failure classification and repeated activation repair. Visual
verification of this update on a physical device remains outstanding.

Startup preparation is process-owned and single-flight. A recreated Activity
joins the same preparation. Admission reopening triggers fresh self-target
preparation. A paused lane leaves the read interface available; the app never
reboots the device itself.

Logging capture acquisition/release is ordered with all writes. Effective debug
is fresh user intent OR at least one active token. Release updates token ownership
even while writes are paused. After successful manual recovery, the coordinator
can submit a fresh logging reconciliation if the released token left debug on;
this is distinct from replaying the interrupted operation. The switch explains
when a capture is keeping logs enabled. Forensic capture reservation and bundle
assembly are not migrated by this logging integration.

Full reset has a separate Cleanup phase (core's `FullReset` effect), validates
canonical-file absence, and publishes Missing. Core empties `/data/adb/vpnhide`
except its own `core/` directory. The app does not offer it while a backend
module is still installed, the kernel is still hooked or the LSPosed module is
still active (`ResetBlocker.kt`). It rejects active logging capture and any open
draft. Ordinary queued edits cannot resurrect a removed config; only explicit
startup/import bootstrap may create it. Invalid or inaccessible canonical data
remain a read/recheck gate; they are never silently replaced with defaults.

Bridge results retain `ok`, `changed` and restart advice and add `errorCode`,
`phases`, `conflicts` (path segment arrays), and `draftPending`. `changed` reports
confirmed persistence even when activation or a late UI conflict fails. Results
do not claim that an uncertain phase failed or that unattempted work ran.

## Runtime integration validation (1.x, historical)

The signed release APK built successfully with R8 and release vital lint; its
signature verified, and both ARM64/ARMv7 mutation helpers are packaged. No APK
was installed and no device reboot was performed for this integration stage.

On 2026-09-15, all 624 JVM tests passed (17 added for this integration), with
no failures or skips. Kotlin compilation with `-PvpnhideWarningsAsErrors`, ktlint,
detekt, CPD and Android `lintDebug` passed. New status text covers EN/RU/ZH. The local checks used two Gradle workers
and a separate single-use daemon for lint; no project lint rules were disabled.

The JVM suite covers capture/user intent ordering, release during paused recovery,
reset readback and lock-inode preservation, editor merge/acknowledgement, bridge
phase/conflict reporting, presentation decisions and joined startup after waiter
cancellation. Root dispatch tests use controlled coroutine gates, and reset tests
run the actual shell builder against isolated temporary paths.

Device acceptance remains outstanding: install the signed APK, exercise first
adoption, root authorization delay, activity recreation,
bridge/UI conflicts, and capture cleanup. Host tests and APK compilation do not
establish app-originated root permissions or visible behavior on a physical device.

Observation generations have since been connected; see the
[observation coordinator](observation-coordinator.md) for their publication boundary.
Diagnostic invalidation, eligibility, execution and capture reservation must follow the separately reviewed
[diagnostic semantics](notes/diagnostics-state-analysis.md) and transition contract.
