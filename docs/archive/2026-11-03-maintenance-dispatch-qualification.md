# Existing-Core replacement dispatch qualification — 2026-11-03

Historical engineering evidence for the implementation committed with this file.
Current behavior and remaining work live in [maintenance](../core-maintenance.md)
and [ROADMAP](../../ROADMAP.md). This is not Android manager/device acceptance.

## Implemented slice

RPC27 / journal13 (reads1–12), retaining kernel control2 and telemetry1.
An explicit existing-Core replacement uses the existing durable command writer:
authenticated full ZIP/private-copy preparation outside the writer, immutable
source/manager fences, floor acceptance, sync-before-dispatch, bounded retained
ELF manager execution, staged-image qualification, guarded legacy marker
retirement and read-only post-boot running-image verification. Active work and
the last maintenance receipt survive normal receipt pruning. Historical Unknown
receipts are not rewritten by later generation proof. No installer replay or
reboot occurs in recovery. Unverified replacement work also fences ordinary
config/runtime Repair admission; reads and original receipt queries stay live.

APK bindings expose typed progress and an explicit bounded SAF copy for its
signed embedded target. A lost acknowledgement queries the same operation;
unknown admission cannot allocate a replacement operation. Native generation
confirmation requires current Core state. Save/Cancel and drafts are unchanged.
No backend publication/selection/fallback policy is changed by this slice.

## Checks on final source

- Selected eleven-package Rust suite:688 passed,0failed,0ignored. This includes
  full archive adversaries, floor/source corruption and cache retention, actual
  command-writer admission/deduplication, manager subprocess and post-boot image
  controls, schema migration, existing RPC/FFI and runtime regressions.
- Five real child-Core exits at maintenance admission, before/after dispatch and
  before/after retirement. Reload preserves operation identity/holds and never
  reruns the manager. A test-only independent ELF manager reads the real retained
  package FD and exercises missing/partial image and live-code mutation; exit0
  never substitutes for image evidence.
- Six final manager-boundary tests include AOSP `/data` ownership versus the
  strict `/data/adb` boundary, discovery alias changes, executable replacement,
  custom installers and ambiguous/escaping candidates.
- Full JVM suite:580 passed,0failed,0errors,0skipped. ktlint, detekt, CPD and Android
  lint passed. The new SAF regression rejects short/long documents without
  replacing prior bytes. The publication binding freezes maintenance status.
  The real host FFI library bytes participate in the JVM task cache key, so a
  shared Rust policy change cannot reuse stale Kotlin/native test results.
- Strict all-target/all-feature host Clippy and both ARM64/ARMv7 Core/FFI Clippy
  passed; fmt, module scaffold and APK no-root-exec checks passed.
- Final startup repeat:5 real-child snapshot-admission tests,3 IPC startup tests
  and24 JVM startup/session/retry tests passed. Competing startup snapshots wait
  within bounded admission; cancellation/expiry never dispatches a late child;
  a running child's permit survives caller cancellation. Failed/truncated reads
  remain failures. This addresses ordinary-ordering regressions behind
  `core root snapshot probe exit=-1`; the user's old phone APK was not tested.

Actual APK native libraries (uncompressed bytes; all ELF LOAD alignments and
file-offset/virtual-address congruence checked at16KiB):

| ABI | Bytes | SHA256 |
| --- | ---: | --- |
| arm64-v8a | 2375240 | `61187e8aef5995b2f0b10cd73f31363e9f699b206ed0b9fdbe26ca81bfc6d970` |
| armeabi-v7a | 1435528 | `ff1fdbcec0071975c226b5ab0381ab881075572613014d88bbc193f374ae42af` |

Cloud logs and source hashes are retained under
`/workspace/vpnhide-build-evidence/maintenance-dispatch`. These are not a signed
production APK or a same-commit full Core bundle. Positive host fixtures use
independent synthetic payloads and test-only publisher identities; production
compiled trust is not replaced with a temporary key.

## Remaining code and external gates

Missing-Core/incompatible-RPC no-su bootstrap/manager handoff, authenticated
artifact retrieval/update scheduling and richer actionable refusal UI remain
implementation work. RFC2 routes/rules/proc-sys/sysfs/framework callbacks and a
genuine32bit worker, export/agent orchestration and notifications remain separate
approved work; this commit does not claim them complete.

Actual Android manager/fork/SELinux behavior, real reboot/readback, old hooked
processes, production CI signing and the final same-commit full bundle remain
unverified. No phone/provider operation, production signing, release, boot-image
write or automatic reboot was performed. Next authorized device acceptance should
use a compatible paired APK/Core, first verify cold startup/reconnect/Retry, then
explicitly stage an authenticated update with the real manager and verify the
running image after a user-chosen reboot. Existing builtin compatibility is not
invalidated merely by the RPC/journal version increment.
