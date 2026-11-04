# App configuration intents

Current 2.0 contract. [Core architecture](core-architecture.md) describes the
privileged owner; this document covers app-local intent, drafts and feedback.

## Ownership

`CanonicalConfigRepository` owns one process-lived `ConfigCoordinator`: its
intent queue, pending toggles, registered drafts and one completion per app action.
It observes the shared typed `CoreSession` publication. Core owns durable commands,
phase plans, receipts and restart recovery. The old `CoreConfigIo`,
`ConfigCoordinatorIo`, app phase executor and recovery reducer are removed.

A cancelled waiter detaches without cancelling the action. Activity recreation
keeps the process queue. Local preferences remain in DataStore; app-context
measurements have their own owner.

## Typed edits and drafts

Screens submit field intent or a pure transform. Preparation reads fresh core
state, derives the write set and protects overlapping registered drafts.
`canonicalEdits` preserves unrelated roles and settings. `RemoveApp` has a broader
write set than removing a role. Explicit import uses `Replace`; ordinary edits
become an atomic flat `Batch`.

The coordinator distinguishes its local action ID from the durable core-issued
operation ID. One `CoreCommand` carries the operation ID and base config revision.
Core merges unrelated changes against fresh state and refuses stale overlapping
fields. Kotlin never sends a screen snapshot as an ordinary full replacement.
The pure reducer, field diff/write paths and candidate validation live in
`core-rpc::config_edit`. Local generated UniFFI calls use that implementation for
draft preparation; Kotlin only maps UI records and retains draft/intent state.
Explicit same-value writes remain intent even when the field diff is empty.
These calls perform no socket IO or effects. Core repeats validation and capacity
admission against its fresh state before journaling or persistence. Save/Cancel
and recovery of an uncertain submitted operation remain unchanged.

Role-set reconstruction, picker Save projection and manual/automatic hidden-set
reconciliation live in `core-rpc::config_selection`. Android supplies package
signals and scan completeness; local FFI returns the candidate without a root
request. Missing rows retain independent roles and overrides. A partial scan
retains unseen auto-hidden members; observed packages are recomputed against the
current rules. Re-enabling a role resets stale hook overrides, while roles that
stay enabled retain their overrides. These projections preserve `debugSwitch`
independently of effective capture logging.

Ports rules, preset materialization and agent policy intents share
`core-rpc::port_policy`. The disk codec uses the same checked range parser and
normalizer; UI/agent adapters call it through local typed FFI. Normalization
removes only exact duplicates, preserving equal TCP/UDP ranges. Preset identifiers
in stored config do not regenerate or replace its explicit rules. All/Preset
agent intents ignore unrelated custom-rule arguments as before. These previews
perform no RPC or effects; the writer independently rejects invalid candidates.
The editor retains its text drafts, formatting and confirmation UI.

Editor ViewModels retain drafts through recreation. Acknowledgement removes only
submitted revisions, preserving typing during Save. A draft registered after
dispatch remains visible and is reported as an overlapping UI edit; it cannot
cancel accepted root work.

## Results and uncertainty

Core receipts describe Persist, Secret, Native, Ports, cleanup and reset phases as
applicable. Confirmed persistence remains saved after activation failure. UI phase
maps describe these receipts and never dispatch a second sequence of effects.
SuperKey changes, import plus legacy cleanup and FullReset are single durable core
commands. Secret values are not stored in the command journal.

`PrepareSelfTargets` is a fixed Core-derived command shared by production boot
and APK startup recovery. It preserves existing user choices, requires complete
legacy observations for first bootstrap, and does not clean up legacy sources.
The app registers broad draft protection but sends no canonical replacement.
An uncertain preparation has the same operation-query semantics as other commands.
Its original base revision fences a concurrent reset/replacement; it is never
rebased after reading legacy inputs.

A lost submission response is recovered by querying the same operation ID, even
when publication advanced past its in-flight state before the app observed it.
No automatic resubmission under a fresh ID is permitted. Unavailable readback does
not turn uncertain dispatch into known failure. A held resource blocks ordinary
edits, but explicit repair can supersede it where fresh delivery is confirmed.
Original Unknown receipts stay Unknown.

Credentials remain saveable after a known activation error or capacity refusal.
FullReset rejects open drafts in the app and checks actual recorder activity and
capture leases in core. It preserves the operation journal and ownership locks.

## Publications, logging and capture

The publication updates confirmed config and effective app logging. One reducer
fences connection generations, request ordering, epochs and revisions. Current and
last-good state remain distinct; disconnect never becomes empty config or disabled
protection.

Core owns generic capture leases and restores the previous logging intent after
the last release/expiry. Forensic actions run in process scope, renew leases and
release in a noncancellable finalizer. Logcat recording survives APK death. The
journal's restoration obligation survives recording acknowledgement/deletion.
There are no app-local logging-token writes alongside core's writer.

Configuration feedback has one owner across navigation: pending switches, quiet
success, failure Snackbar, activation repair and a read/recheck gate. Desired
state, delivery evidence and actual diagnostic measurements stay separate.

Device-local next-boot kernel selection uses `StageBackendChoice` through this
same coordinator and operation-ID recovery path. It has no canonical edits,
protects drafts during preparation, and reports the actual Core Maintenance
receipt. Saving a choice is distinct from applying a backend; the publication
shows PendingRestart or Unverified separately from current confirmed delivery.
No automatic restart or optimistic per-toggle Save change is introduced.
