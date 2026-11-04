# Resident kernel configuration continuity

Date: 4 November 2026. Implements one part of
[RFC0005](../rfcs/0005-unified-backend-lifecycle-policy.md), not the full explicit
backend choice or legacy Zygisk lifecycle.

## Behavior

Production bundle preparation distinguishes an existing positively identified
kernel endpoint from a new load. A resident Builtin, KMOD or KPM can proceed to
the usual journaled configuration delivery when fresh same-backend status and
the complete projected policy are compatible. Replacement KMI ambiguity, unknown
kernel configuration, disabled module loading and the Core executable ABI are
new-load constraints; they do not request a new load or invalidate this endpoint.

Missing, fatal, unknown or wrong-backend status cannot use compiled replacement
capabilities. PartialHooks retains the actual mask. Required hooks, target/wire
limits, default-mask support, ownership, journal holds, dispatch reobservation
and possible Zygisk exposure remain enforced. No family fallback is added.
Adoption does not attest the resident image hash or claim behavioral PASS.

## Executable checks

- Pure assessment checks all three resident kernel families despite unavailable
  fresh-load evidence, and refuses the equivalent fresh selection. Missing
  capability evidence, unresolved policy, missing hooks and exceeded target/wire
  limits still refuse reconfiguration.
- Production catalog-to-prepared tests retain exact `Adopt` intent with no
  payload for compatible Ok/PartialHooks endpoints; missing/fatal/unknown hooks
  do not become permission. Fresh-load evidence is deliberately unusable.
- Wrong-backend telemetry is rejected. Missing resident support cannot borrow
  compiled capabilities. Ambiguous replacement KMI retains its new-load refusal
  while allowing a separately verified compatible resident endpoint.
- Full Core tests: 488 passed, no failures or skips, including startup,
  writer/recovery, RPC and real child-process regressions. Host all-target Clippy
  with the fixture oracle and strict ARM64/ARMv7 production Clippy passed.
- Rust formatting, module scaffold and APK no-root-process checks passed.

RPC32, journal15 and backend control2 are unchanged. No JNI/Kotlin contract changed.
No phone, provider settings or boot writes were used. Actual device continuity,
fresh same-commit full distribution and production signing remain separate gates.
