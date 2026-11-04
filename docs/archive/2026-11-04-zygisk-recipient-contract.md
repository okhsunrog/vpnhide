# Shared Zygisk recipient contract and kernel-capable unit runner

Date: 4 November 2026. A boundary cleanup for the remaining
[RFC0005](../rfcs/0005-unified-backend-lifecycle-policy.md) lifecycle work.

## Code

`protocol::zygisk_activation::recipient_from_manifest` is the only module-property
grammar used by both the Core writer and the Zygisk pre-specialization reader.
The bounded UTF8 parser rejects missing, duplicate and unknown critical fields.
The existing authorized compatibility recipient requires control2 and activation1;
an otherwise valid authorization cannot override a missing/unsupported control
contract. Only the explicitly recognized legacy control2 recipient without an
activation marker can read `targets.txt`.

This removes the weaker library-side control check and the two separate parsing
loops. It does not publish a module, grant consent, infer installed hooks or
finish managed projection/explicit backend selection. The original Core module
identity is retained for authorization/revocation compatibility. Production
distribution still contains one passive Zygisk ABI pair and no Core autoload.

The Linux musl unit build also exposed a native ABI assumption: `msg_iovlen` is a
signed int there, whereas bionic/glibc expose `size_t`. `hooked_recvmsg` now uses
one checked adapter before touching descriptors. Negative signed lengths refuse;
valid Android counts retain the same behavior. Existing scatter/gather bounds
and filtering policy are unchanged; no unchecked cast or relaxed lint was used.

## Checks and exact limitation

- Full Core/protocol/tooling host invocation: **550 tests passed**, zero failures
  or skips. Final portable tool/protocol repeat:55 passed. Targeted GNU Zygisk
  reader tests:5 passed; the checked-count regression:1 passed.
- Strict host all-target Clippy includes Core's oracle, Zygisk, protocol and the
  tooling. Strict Android API28 ARM64/ARMv7 Core/Protocol/Zygisk production
  libraries/binaries passed. Rustfmt, scaffold and no-root-APK checks passed.
- The ordinary cloud kernel reports `process_vm_readv(self) = -1 / ENOSYS`.
  The initial complete GNU Zygisk suite therefore had51 passed/2 failed: the
  valid-pointer self-copy control and the libc-hook caller-memory control.
  No production hook was weakened and these failures were not called a pass.
- A dedicated typed `userspace/native_unit` QEMU scope runs the exact hashed
  native test executable with the full suite and a bounded process deadline.
  Its report cannot masquerade as Full/fixture proof or claim loaded hooks.
  On the reference Linux6.1.175 Image, the final AArch64 musl Zygisk executable
  SHA256 `1069b4f91e5295bf665572b365f0cc0d2362d6c64063e3e7b569448fb1b50965`
  passed **all54 tests**, with zero failures, ignored or filtered cases, including
  both real caller-memory controls. Session
  `bf767307e06aa700885989dbb3fb7d744c77eae17bb3e53208d91f9fe52d9709`.
  No backend was activated by this suite runner.

External evidence is in `vpnhide-build-evidence/zygisk-recipient-contract`, with
logs, exact native executable, typed plan/input identities and QEMU serial data.
The existing clang17 static-link unused `-no-pie` warning is retained, not hidden.

RPC32/journal15/control2 are unchanged. QEMU Linux musl is not Android bionic,
provider lifecycle, SELinux or genuine secondary32 worker acceptance. Existing
Zygisk continuity, desired/effective choice, voluntary post-update offer,
managed discovery projection and actual device verification remain separately
tracked code and acceptance work in [ROADMAP](../../ROADMAP.md).
