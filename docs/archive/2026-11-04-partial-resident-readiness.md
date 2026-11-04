# Partial resident configuration readiness

This is an unpublished local RFC0005 implementation slice. It does not authorize
a main push, another backend family, a release or device operation.

Bundle preparation used to replace the installed-hook mask with zero for every
nonzero status. KPM configuration already accepted `partial_hooks` as nonfatal.
That discrepancy could hold a resident even when its actual installed hooks
covered the desired policy.

The shared protocol status now owns configuration-error readiness: only Ok and
PartialHooks permit the remaining configuration checks. KPM delivery consumes
that owner. Core preparation preserves the literal installed mask for those
states, and supplies no support evidence for fatal/unknown errors. It does not
fabricate zero-hook telemetry or infer readiness from a nonzero mask.

Core still checks the complete projection, required hooks, capacity, wire size
and default-mask support. A partial resident that covers the policy is prepared
for adoption, with no new payload/load. Missing required coverage or unknown/fatal
status refuses preparation without selecting another family. Native health and
process/behavioral proof remain distinct; no PartialHooks result is promoted into
overall PASS by this change.

Regression cases cover Builtin, KMOD and KPM with an installed required hook,
missing required hook, all defined status codes and an unknown code. The shared
wire classifier additionally checks empty/full masks cannot override error class.
407 Core library tests,63 activator library tests and21 protocol tests passed,
with no failures/skips. Strict host all-target Clippy including the fixture oracle
and strict ARM64/ARMv7 checks of all three libraries passed. Formatting and local
documentation file links passed. Logs are retained under
`/workspace/vpnhide-build-evidence/backend-policy-partial-resident/`.

The final binary also passed all11 targeted process regressions:3 IPC startup /
reconnect,2 capacity refusal,5 self-target/bootstrap and1 real standard-module
probe dispatch. These are cloud regressions; the Android startup screenshot still
needs acceptance with a current paired APK/Core on the phone.

Cross-upgrade continuity, explicit backend settings and warning consent, voluntary
Zygisk migration offers, durable/public passive selection evidence, and final-file
preflight dispatch remain code work. This regression is not completion of those
flows or acceptance on a real Android device. See [ROADMAP](../../ROADMAP.md).
