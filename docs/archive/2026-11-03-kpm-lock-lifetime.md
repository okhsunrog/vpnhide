# KPM scoped lock lifetime — 3 November 2026

Engineering correction after `11ffdb789640451780c04da33148672ec7426444`.
The working tree also contained the package-inventory codec migration during the
integrated checks; this is not a claim of an isolated release build or device test.
Exact source hashes and logs are retained under
`vpnhide-build-evidence/kpm-lock-lifetime/qualification.json` in the execution workspace.

A final host run failed `delivery_waits_for_local_observation_and_dispatches_once`.
The local observer had ended, but an unrelated child could still hold the inherited
open file description before CLOEXEC. Closing the parent's File alone did not
release its flock. A new deterministic test pauses that child, releases the owner,
and tries delivery admission. It failed before the fix with `WouldBlock`/errno11,
without dispatching or relying on load/timing guesses.

KPM control now uses the same process-owned scoped FileLock as Core instance,
maintenance floor and staging guards. The implementation and child-drop test were
moved from Core into the portable bounded-process library; no parallel helper was
added. Owning-process Drop explicitly unlocks before releasing the local KPM gate.
A fork child's copied guard never unlocks its live parent. Existing blocking and
nonblocking admission modes remain, including definite refusal on external
contention. No retry, fallback, runtime-unload or backend policy was introduced.

The new regression and the original delivery test pass after the fix. The final
combined six-package run passed639 tests with zero failures or skips; it includes
maintenance inherited-FD regressions, child-drop exclusion, external KPM contention,
single delivery and Core process/startup recovery. Strict host/all-target and both
Android ARM library Clippy checks passed. Snapshot codec/FFI integration is recorded
separately with that migration; neither this helper nor ARM compilation demonstrates
a genuine secondary32-bit Android worker. RPC31, journal14, diagnostic9 and debug
schema11 are unchanged. No phone, production key, boot write or release was used.
