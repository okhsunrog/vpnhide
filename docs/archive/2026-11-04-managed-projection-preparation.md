# Inert managed projection preparation — 4 November 2026

This is a format, Root preparation/readback and loader-contract implementation
slice under accepted RFC0005. It is **not** a production projection dispatcher,
new Zygisk selection flow or device/provider acceptance.

The shared artifact model emits exactly module metadata, a bounded descriptor and
one final library for each ARM ABI. Metadata identifies only
`vpnhide_zygisk_runtime` with control2/activation1/projection1. The descriptor
binds the operation, exact source commit, normalized and raw manifest digests,
and both payload identities. No authorization, legacy targets, service,
installer script or independent update channel is emitted. Core ZIP dedup is
unchanged; no real device projection was created.

The preparation component verifies the actual running Core inode and complete
catalog, enabled Core identity, checked manager and absence of existing targets.
It copies immutable final bytes, constructs and seals the full archive, and
removes its private temporary files before admission. `require_fresh` rechecks
source, running inode, manager and output absence before dispatch. Readback checks
both ABI hashes and ELF headers, directory/file ownership and modes, link types,
closed layout and exact descriptor. A staged image, full active copy and active
metadata placeholder are distinct. A placeholder alone, extra code, partial ABI
pair or successful manager exit alone is not readiness.

Two real preparation bugs were caught by executable regressions: cloning the
sealed FD retained its EOF cursor (fixed with offset reads), and reusing metadata's
64 KiB bound rejected real-sized libraries (fixed with explicit permitted large
files). Original failing logs are retained. No lint suppression or weaker layout
gate was added. Format validation reuses the existing ELF/manifest owners.

Core and the library share the exact Managed recipient grammar. Managed policy
never falls back to legacy targets. Availability requires current-boot authority
and observed exact generation/policy consumption. An empty bootstrap may unload
in a worker: that is only loaded-code/consumption evidence. Protected-worker joins
still require the actual live hook image. Both ABI proofs remain required on
mixed devices; the genuine secondary32 Android launcher remains unimplemented.

Final cloud verification:

- 576 Rust tests: 517 Core including integration/crash/startup, 36 artifact-model,
  23 protocol; zero failures or ignored tests.
- Seven preparation tests, including a real separate ELF manager adversary,
  sealed-input write refusal and independent incomplete/foreign/unsafe outputs.
  Thirteen loader tests retain exact source/recipient/generation boundaries.
- Actual AArch64 QEMU NativeUnit execution: all 55 Zygisk library tests passed,
  zero ignored/filtered. Session
  `fe9886603a5fafe5495dbd1c9dd31765ccd010edecd3fb5cf9d286a244b0509c`.
  This exercises the musl test binary on the pinned reference kernel, not bionic,
  Zygisk provider or Java hooks. The cloud GNU host's existing process-memory
  ENOSYS limitation was not waived or hidden by ignored tests.
- Strict host all-target Clippy including the fixture oracle and API28 ARM64/ARMv7
  Core, Zygisk and APK FFI libraries/binaries passed. fmt, module scaffold,
  no-root APK boundary and whitespace checks passed. An initial cargo-ndk flag
  syntax failure was corrected; it is not counted as a successful gate.

Evidence, exact source/log hashes, the QEMU executable, session inputs and results
are retained at `/workspace/vpnhide-build-evidence/managed-projection/`.
RPC33/journal16 and kernel control2 are unchanged. No Kotlin/generated FFI shape
changed; previous JVM/JNI qualification is separate, not rerun or relabeled here.
No new production signing, final bundle, phone, provider settings, manager install
or reboot occurred. Push bypass of the repository's ci-ok rule is not verified CI.

Remaining code includes journal-bound dispatch/recovery, next-boot reconciliation,
bootstrap authority and worker launch, current recipient routing, owned updates
and withdrawal, existing-user continuity and the voluntary kernel migration offer.
Production still refuses new Zygisk choice/current-Zygisk retirement until that
controller exists. No automatic fallback or standby publication is introduced.
