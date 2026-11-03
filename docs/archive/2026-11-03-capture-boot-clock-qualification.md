# Capture logging limits across suspend — 3 November 2026

Engineering slice after `b141375c5feaebad4f1fb6d9ce48f156829514b5`.
Exact changed-source hashes, logs, JVM reports and real FFI ELF headers are retained
under `vpnhide-build-evidence/logging-boot-clock/qualification.json` in the execution
workspace. This is cloud evidence, not Android suspend acceptance or a release.

## Corrected production paths

Temporary forensic/logcat logging leases previously used `Instant` for authority,
and logcat used it for the recording cap and elapsed time. Android/Linux suspend
does not advance that clock, so sleep could extend the intended lifetime.

Core now reuses the checked CLOCK_BOOTTIME implementation already used by autonomous
diagnostics. It lives once in `core-daemon/src/boot_time.rs`; the old diagnostic
copy is removed. Both logging lease owners and the existing Core-owned recorder
consume it. No second recorder, APK timer owner or root execution path was added.
The existing limits remain five minutes for a forensic lease,31 for a recording
lease and30 for the recording itself; explicit lease renewals remain bounded.

Lease admission obtains its bound before persistence/effects and rechecks it after
persistence. Unknown/regressed clocks cannot grant fresh authority. Expiry follows
the normal durable restoration path, respecting held resources, storage faults and
the user's current debug switch. Journal wall-clock expiry remains display metadata.

The recorder rechecks its boot deadline with active waits of at most250ms. A long
suspend does not restart the full remaining timeout after resume. No timer wakes
the phone or claims cleanup can execute while its CPU sleeps. An unavailable or
regressed clock stops the child with `ClockUnavailable`; it is not mislabeled a
normal cap or Core restart. Duration retains the last valid measured value and can
be a lower bound. Existing child reap, file retention, debug restoration, export,
ACK and discard consumers remain the owners of those actions.

## Compatibility and executable coverage

Paired APK/Core RPC31 adds the typed stop reason through Rust, FFI and localized
Android presentation. Existing field names stay stable; debug schema11 records
the changed duration meaning. Private journal14 also fences older writers from the new persisted
recorder reason: Core startup writes the upgraded journal before recorder recovery.
Schemas1–13 still migrate with holds, budgets and receipts preserved. Older Core
builds must refuse the newer journal; deleting it is not a downgrade procedure.
Kernel control2/telemetry1 is unchanged and these numbers require no builtin rebuild.

The retained gate covers696 Rust tests across the changed packages and artifact,
maintenance and host-tool consumers, plus588 JVM tests. Matrix tests advance boot
time without advancing an application monotonic clock, cover exact expiry,
regression and unavailable clocks, and exercise restoration through the real durable
writer. Independent persisted `clock_unavailable` input survives Core restart,
preserves its measured duration and file, and remains deliverable/acknowledgeable.
Existing real-child recording, cap, restoration, restart and storage-failure tests
remain. Typed RPC/FFI/JVM boundaries retain the distinct reason.

Strict host/all-target Clippy including the fixture oracle and both Android ARM
library targets, fmt, Kotlin checks and real FFI16KiB LOAD/congruence checks are
recorded with the slice. An earlier overlapping host rebuild invalidated a doctest
dependency; it is not counted. The final serial gate passed without that overlap.

Real Android suspend, background execution, SELinux and callback acceptance still
need a device. A genuine secondary32-bit Android worker on a mixed-ABI device is
still an open implementation gap; ARMv7 compilation is not execution coverage.
Capture/export and agent orchestration transfers remain separate code work. New
backend policy remains paused. No phone, production key, provider setting, release
or real boot write was used.
