# Native RAW worker integration — 2026-11-03

Historical cloud evidence following provider commit
`fc6ad14253ce4a866675f2f91a950e9bfc574c9b`. This records the implemented Core/Android
consumer slice, not completion of RFC2 or real Android acceptance.

## Implemented flow

The genuine `:diagnostics` service owns one collector executor thread. Native
self-registration and both synchronous observations stay on that thread across
RPC suspension. Core revalidates the Zygote worker, task membership/birth and
PID/user/net namespaces before each phase and before final evidence acceptance.
A prepared attempt is reserved before kernel IO; replay cannot select a fresh
nonce or repeat activation. Only actual absence/unsupported capability permits
protected-only collection without a grant. Unknown control outcomes never
become a guessed protected sample.

Core confirms the bounded RAW interval and revocation independently. A completed
RAW observation under expiry or unknown cleanup is retained as failed evidence,
not as a protected leak or successful proof. Shared evaluators require both root
controls and same-worker RAW visibility for native proof. Independently qualified
protected leaks survive missing RAW or later controls. The root-only finish entry
point is compiled only for the explicit fixture oracle.

Protected observations reach Core **before** framework collection. A separate
completion ACK ends the worker; an accepted native observation remains available
when Java fails or the worker dies/times out. Completion revalidates the exact
collector; failed, replayed or stale completion does not erase observations.

Every RAW receipt retains UID/PID/TID and both birth ticks, including unsupported
and failed receipts. `paired` describes scope correlation/revocation, not positive
stimulus or PASS. Schema8 exports the full retained receipt using the shared Rust
codec, rather than a second Kotlin measurement codec. Kotlin performs bounded
observations and vocabulary/protocol adaptation, not verdict or grant policy.

The paired contract is RPC28/diagnostic contract7; journal13 still reads1–12.
Kernel control2/telemetry1 are unchanged. These version numbers alone require no
builtin rebuild; an existing kernel without the optional RAW provider remains
compatible and its native diagnostic proof stays unverified.

## Cloud verification

- 588 selected Rust tests, zero failed/ignored, including transport/process,
  migration/security and diagnostic regressions. Thread/namespace revalidation,
  replay/ACK loss, early evidence retention and late cleanup failures have
  executable coverage.
- 584 JVM tests, zero failed/skipped; actual collector-thread continuity across
  suspension, protected-only flow, unknown ACK refusal and forensic attribution.
- Strict host all-target Clippy and both Android ARM library Clippy; no blanket
  lint exclusions. The large measurement variant uses transparent Box support.
- Actual ARM64/ARMv7 FFI libraries built, each with four LOAD segments aligned to
  0x4000 and congruent virtual/file offsets. Kotlin formatting, detekt, CPD,
  Android lint, module scaffold and APK no-su checks passed. Schema8 golden was
  regenerated through the normal test task.

Machine-readable source/artifact hashes, test reports and readelf evidence are
retained in `/workspace/vpnhide-build-evidence/rfc2/native-raw-worker/`.

## Not established here

No phone, actual builtin boot, bionic/SELinux/background-service acceptance,
provider settings, production signing, release or reboot was performed. The
previous Linux provider oracle does not become Android evidence by association.
KPM/Zygisk RAW adapters, a genuine mixed-ABI 32-bit worker mechanism, remaining
route/rule/proc/sys stimuli, framework default/callback/PendingIntent scenarios,
complete process-generation coverage and the rest of RFC2 remain separate work.
This change adds no backend selection/publication/fallback policy and no full
suite-success variant. Current tracking belongs to [ROADMAP](../../ROADMAP.md).
