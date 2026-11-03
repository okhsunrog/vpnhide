# Clean native bundle and unsigned APK — 3 November 2026

Historical source checkpoint: **`21cd31eedce06cc0e6c48b401da8d425c56c19e5`**.
The checkout stayed clean throughout packaging. This is scoped cloud build and
regression evidence; the architecture refactor and device acceptance are unfinished.

## Actual artifacts

All ten producer archives report that same commit and `dirty=false`. The shared
Rust assembler and descriptor structure verifier accepted the full Core ZIP:

- **5,335,293 bytes**, SHA256
  `e1413d0093faf03846a4fdfb0b594cafc2264a0e5154669501a5ac7200ed1fe0`.
- Twenty ZIP entries, ten payloads: seven KMI modules, one KPM, two Zygisk ABIs.
  The two daemon ABIs are separate from that payload count.
- Exactly one passive Zygisk pair under `payloads/zygisk/`; no main-Core autoload
  directory. No experimental KPM RAW adapter was included.

| Native entry | Uncompressed bytes | ZIP compressed bytes |
| --- | ---: | ---: |
| Core ARM64 | 4,866,088 | 2,406,030 |
| Core ARMv7 | 3,314,316 | 2,095,276 |
| `.ko` android12-5.10 | 114,360 | 34,100 |
| `.ko` android13-5.10 | 115,408 | 35,146 |
| `.ko` android13-5.15 | 124,400 | 38,495 |
| `.ko` android14-5.15 | 141,520 | 41,596 |
| `.ko` android14-6.1 | 116,888 | 36,949 |
| `.ko` android15-6.6 | 99,856 | 33,137 |
| `.ko` android16-6.12 | 111,176 | 36,167 |
| KPM ARM64 | 100,872 | 30,979 |
| Zygisk ARM64 | 547,416 | 282,623 |
| Zygisk ARMv7 | 397,808 | 258,460 |

The release APK passed Kotlin/Java compilation, shrinking and R8 **with signing
disabled by a temporary build-model adapter**. No repository signing setting or
keystore was changed. `apksigner` independently rejected it as unsigned.
It is not the production-signed or installable device acceptance APK.

Unsigned APK: **9,880,481 bytes**, SHA256
`27d2c62526132a264ad4af8f93388ed605b8f74e62768294044ebb412e9d5421`.
It contains eight native libraries: ARM64/ARMv7 pairs of Core FFI, JNA,
AndroidX graphics-path and DataStore shared-counter libraries.

## Qualification and recovery

All seven builds used the previously recorded immutable DDK image digests and
their own compilers. Actual final ELF imports/CRCs and hash-bound init/preflight
capabilities passed the shared `require-gki-contract` verifier. This proves
structural loader inputs, not private ABI compatibility on a phone.

The last image initially exhausted tmpfs. The failed extraction log was retained;
only task-owned reproducible compiler/test caches and the incomplete extraction
were cleared. Its retry omitted unused Rust, alternate musl toolchains and static
toolchain archives. Kernel sources/headers, config and symvers were preserved;
the normal compiler and strict checker completed successfully. The first six
qualified outputs were not rebuilt or replaced during this recovery.

Actual NDK LLVM program headers were checked for twelve executable/shared
images: both Core and Zygisk ABIs plus all eight libraries from the final APK.
Every LOAD uses 0x4000 alignment with congruent virtual/file offsets. Relocatable
`.ko`/KPM checks are separate; this does not establish Android page-size acceptance.

The unchanged runtime code had **591 selected Rust tests**, strict host/ARM64/
ARMv7 Clippy and scaffold/no-su checks. The subsequent clean APK run repeated
**38 targeted JVM startup/session/root regressions**, zero failures/skips,
including explicit probe failures, coalescing, reconnect and Retry. Earlier full
584-JVM qualification remains pinned to the RAW integration slice. These are
selected suites, not an asserted repository-wide test count or the final check
after all remaining implementation work.

## Retained evidence and remaining gates

`/workspace/vpnhide-build-evidence/arm-release-21cd31ee/` contains the full ZIP,
producer archives/provenance, exact native inventories, actual ELF reports,
unsigned APK and its negative signing verification, JVM reports and build logs.
The unsigned offline descriptor uses a synthetic sequence solely to exercise
structure verification; it is not authenticated release metadata or a rollback floor.

No production signing, release, phone/provider action, boot write or reboot was
performed. RPC28/diagnostic contract7/journal13 and kernel control2/telemetry1
remain unchanged. New backend policy is still paused. KPM RAW lifetime/transport,
genuine secondary-ABI worker, remaining RFC2 fixtures, bootstrap, export/agent
orchestration and notifications still include **code work**, separately from
device/key/publication gates. Rebuild later final code; this checkpoint does not
finish [ROADMAP](../../ROADMAP.md).
