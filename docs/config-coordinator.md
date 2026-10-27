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
App validation improves feedback; core independently validates admission.

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
