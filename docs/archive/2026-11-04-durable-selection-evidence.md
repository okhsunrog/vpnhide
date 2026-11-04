# Durable selection observations and KMI conflict

The maintainer approved verified policy publication to main on4 November2026.
This slice implements part of [RFC0005](../rfcs/0005-unified-backend-lifecycle-policy.md),
not the complete continuity, explicit-choice or preflight flow.

## Runtime path

Bundle assessment captures bounded kernel/core ABI, uname, manifest identities,
candidate KMI names and live-bound module metadata. Successful preparation freezes
the record in its exact transition intent. Declined preparation carries the record
to the command writer, which syncs it before publishing the refusal. Startup and
explicit Repair use the same owner. No observation is itself a load/delivery or
behavioral PASS.

The journal stores latest observation separately from a retained current-boot
conflict. A later absent/partial inventory remains visible without replacing that
conflict. Admission and conflict replacement recompute module corroboration with
the same Core KMI classifier used for initial selection; a claimed `module_hint`
without distinct supporting names/hashes/vermagic cannot clear it. Existing
startup, preflight, quarantine, exposure and attempt-budget holds are preserved.
This hold applies to a new KMOD load; it does not silently replace another working
resident backend.

Prepared now freezes the actual manifest file SHA256 in addition to its normalized
digest. Dispatch/recovery validates that identity where available; even a
whitespace-only replacement of a newly prepared manifest refuses dispatch.
Older intents without the new field remain readable under their existing fences.

RPC32 exposes latest evidence and retained conflict independently through the boot
read model and generated FFI. The existing Kotlin forensic DTO retains the same
Rust-serialized evidence as JSON data, with no second classifier/codec. Absent
fields are omitted; the prior Rust/app and debug-bundle golden fixtures remain
unchanged. Evidence is bounded to128 KiB each; an actual RPC-envelope regression
covers both maximum-sized records together.

## Compatibility and checks

Private journal15 reads1–14, retaining old counters, expiry floors, holds and
load/preflight budgets. New facts under an older schema, corrupt evidence and
future journals refuse mutation; they are never repaired by deletion. APK/Core
must be paired at RPC32. Diagnostic contract9, debug schema11 and kernel
control2/telemetry1 are unchanged. RPC/journal numbers alone require no builtin
kernel rebuild.

Cloud checks cover restart, receipt pruning, incomplete-input retention, bare-hint
refusal, qualified conflict replacement, old-schema migration, identity mismatch,
manifest replacement, real RPC frame size, startup/IPC and typed FFI/export
preservation. Strict host/oracle and both Android ABI library/binary checks run
without lint suppression. Final logs, source hashes and actual JNI ELF identities
are retained under
`/workspace/vpnhide-build-evidence/durable-selection-evidence/`.

Final source checks passed577 Rust tests across Core/RPC/FFI/client and their
process regressions, plus587 JVM tests, with no failures/skips. Strict host
all-target/oracle and ARM64/ARMv7 library/binary Clippy, Rust formatting, ktlint,
detekt, CPD, lintDebug, module-scaffold and APK no-root checks passed. Both actual
rebuilt JNI images have four LOAD segments with16 KiB alignment and congruent
file/virtual offsets. These are build checks, not Android runtime proof.

## Still code, not device gates

Complete cross-upgrade/existing-Zygisk continuity, advanced explicit backend choice
with warning, voluntary post-update kernel offers and real bounded preflight
dispatch/recovery remain. Production experimental preflight admission stays off.
The genuine secondary32-bit Android launcher and remaining RFC0002/framework,
export/agent and notification work remain tracked in [ROADMAP](../../ROADMAP.md).
No phone, provider settings, release or production signing operation was used.
Current paired device acceptance and a fresh complete signed distribution remain
separate from these cloud checks.
