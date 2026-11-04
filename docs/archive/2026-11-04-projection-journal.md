# Managed projection work and recovery — 4 November 2026

This follows the inert image preparation slice at `aa3b11a0`. It implements the
common journal recorder, before-effect dispatch component, restart fences and
read-only physical observations. **The public Zygisk choice/bootstrap flow is
still not connected.** Ordinary startup does not install a projection. This is
not completion of RFC0005 or Android/provider acceptance.

Private journal17 adds one bounded `ProjectionWork`: operation, explicitly warned
request, real boot, exact source/manager/descriptor, sticky dispatch intent,
preparation outcome and checked output. The common recorder atomically couples
confirmed preparation with the device-local preference and receipt. It refuses
rewinds, rebuilt initial intents for retained operation IDs, changed requests and
terminal Unknown rewrites. The record/receipt survive a zero retention budget.
Schema16 preferences and prior holds/counters migrate intact; earlier schemas
cannot contain new projection work. An older writer refuses journal17. Public
RPC33 and kernel control2/telemetry1 remain unchanged; no builtin rebuild follows
from these private version numbers.

Preparation pins the real boot and rechecks it with source/running inode/manager
and output absence after the writer's synced callback. Every persistence failure
stops later effects. In particular, failed dispatch-intent sync never spawns a
child. After possible spawn, manager failure, timeout, truncated output or invalid
readback is Unknown. A final rename may have landed the whole preference/receipt;
a lost acknowledgment does not become a known installation failure.

Restart never invokes the manager. An accepted record without dispatch intent
proves no spawn; a persisted dispatch marker retains uncertainty. Physical
readback can identify prepared bytes without changing the returned Unknown.
Same-boot observations require exact identities. After a different actual boot,
content continuity still requires the real running inode, complete active code
and no pending stage. No result proves provider scan, generation consumption,
hook health or permission to activate. No auth file or exposure marker is emitted
by the preparation executor.

Uncertain work holds Maintenance. Actual Core reset, another backend choice and
independent replacement commands refuse retained work, so they cannot forget a
published discovery copy. Owned withdrawal, combined Core/projection update and
explicit uncertainty resolution are still code work. The existing kernel-choice
UI, Save/Cancel and uncertain Save recovery are preserved.

Verification on the final component code:

- 596 Rust tests across Core, RPC and package-trust passed, including fifteen
  preparation/work cases, real-process crash/startup and root snapshot contention.
  Zero failures or ignored tests. The new cases cover all atomic-write boundaries
  before/after, sealed manager input, new/source-swapped boots, malformed records,
  same-ID fresh-intent replay and immutable terminal Unknown after late readback.
- Real CoreState commands verify reset/choice/replacement refusal for both
  prepared and Unknown work. A separate schema16 migration retains the preference,
  receipt, issued counter and Native/Ports holds.
- Strict host all-target Clippy with fixture-oracle and API28 ARM64/ARMv7
  Core/FFI libraries and binaries passed. fmt, module scaffold, no-root APK
  boundary and whitespace checks passed. Initial private test visibility,
  unreachable-arm and unused-import errors were fixed, not suppressed.
- Both APK JNI images were rebuilt and every actual ELF LOAD alignment and
  offset/address congruence passed the shared 16 KiB minimum. 587 JVM tests ran
  with zero failures/errors/skips; ktlint, detekt, CPD and Android lint passed.
  Build warnings remain in the log. These working-tree builds precede the commit;
  they are not a clean same-commit final bundle or production-signed APK.

Evidence, exact source/log/JVM XML/JNI hashes and ELF listings are retained at
`/workspace/vpnhide-build-evidence/projection-journal17/`. The preceding slice's
55-test QEMU Zygisk library run is separate; it was not rerun or relabeled here.
No phone, production manager/provider changes, production key or release was used.
A push bypass of ci-ok is not verified CI.

Next code: connect the existing writer's admission to this recorder/dispatcher,
current recipient routing, separately journaled bootstrap authorization, actual
ABI workers, owned withdrawal/update, existing-user continuity and the voluntary
kernel offer. The genuine secondary32 Android launcher and other ROADMAP work
remain implementation gaps, not merely device acceptance gates.
