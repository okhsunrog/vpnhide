# Suspend-aware diagnostic deadlines — 3 November 2026

Historical qualification of the change following `1c0a3fee`. Current work remains
in [ROADMAP](../../ROADMAP.md); this is not Android suspend acceptance.

Core previously used `Instant` for the admitted diagnostic run while native RAW
and framework recipients used boot time. Android/Linux monotonic waits exclude
suspend, so a sleeping phone could resume with an obsolete Core run still
admitted and its TUN cleanup delayed by the old remaining timeout.

One checked CLOCK_BOOTTIME implementation now owns Core admission deadlines,
worker phase/report/completion checks, birth-clock conversion and framework time.
Active Condvar waits cap each monotonic sleep at 250 ms, then recheck boot time.
Clock failure/invalid fields deny authority; arithmetic overflow cannot extend it.
The supervisor's idle five-second wake cycle remains separate from authority.

Framework envelopes inherit the original run expiration rather than receiving
another twenty seconds at Java-phase construction. Expired initial phases cannot
publish an envelope, and ACK readback rechecks expiry after identity/file reads.
Expired runs retain observations accepted earlier for attributed assessment and
owned cleanup; expiry does not manufacture successful proof. No timer wakes or
executes cleanup while the CPU is suspended.

Verification on the final source:

- 591 selected Rust tests, zero failed/ignored, including 28 autonomous tests,
  real root-probe/IPC process regressions and transport/migration/security tests.
- Deterministic boot-time advancement expires claim, report and completion while
  retaining accepted native evidence. Clock ranges, conversion, overflow,
  bounded wait slices and inherited framework expiry have executable regressions.
- Strict host all-target/fixture-oracle Clippy and ARM64/ARMv7 library Clippy,
  Rust formatting, module scaffold and APK no-su checks passed.
- RPC28, diagnostic contract7, private journal13 and kernel control2/telemetry1
  remain unchanged. No Kotlin/FFI model change, device action or backend policy
  transition was made.

Logs and source hashes are retained in
`/workspace/vpnhide-build-evidence/rfc2/diagnostic-deadlines/`.
Real Android suspend, bionic/SELinux and background execution remain device gates.
The separate [ABI launcher source review](2026-11-03-diagnostic-worker-abi-launch.md)
records why instrumentation is not the missing safe secondary-ABI launcher.
