# KPM NULL SIOCGIFCONF qualification — 2026-11-03

Engineering evidence for native ARM64 NULL-buffer size projection. This changes
neither backend-selection policy nor RPC26, journal12, kernel control-v2 or
telemetry-v1. No phone, provider settings, real boot writes or release operations
were performed. The production kernel payload remains AArch64-only.

## Reproduced problem and rejected hypothesis

Before this change, KPM compacted the filled ifreq array but returned the original
NULL-query size. On reference 6.1, the same bionic probe observed RAW size/fill/VPN
counts `2/2/1`, then protected `2/1/0`. The old fill/tail-only row accepted that
limited scope; the new mandatory size/fill row rejects it.

A supplementary `inet_gifconf` hook was implemented and tested, then rejected.
It passed on reference 6.1, but the actual Android 12 5.10 Full LTO/CFI Image
returned protected `2/2/1`: the symbol existed, but intercepting that exact entry
did not cover the live probe path. Disabling the original filled-buffer
compactor in that candidate also regressed concealment. An offline scan found no
direct B/BL to the exact entry; this supports an optimization/reachability concern,
but does not establish its complete cause. Symbol discovery is not behavioral
proof. That supplementary hook and its competing filtering path were removed.

Baseline KPM SHA256:
`1a7e4a8705d4da9a5f505f1a280ec0dbdb0f5040f289dee62af3b264d0e65c08`.
Rejected candidate:
`042761d63e0e6866bf8bbe409c6ab533290d5e5cdcb18cbb8254571f2956e8fe`.
Neither is a release artifact. Negative serial logs, plans, input identities,
candidate source/patch and binary are retained with the local build evidence.

## Implemented path

The existing `sock_ioctl` before/after hook retains the actual ioctl file and
continues compacting filled buffers. For a successful native NULL query,
`sock_from_file` supplies the socket and its network namespace. The new walker:

- holds RTNL while traversing device and IPv4 address lists;
- uses `copy_from_kernel_nofault` or `probe_kernel_read`, with no RAW user scratch,
  allocation or global user-copy hooks;
- limits each candidate walk to 4096 combined device/address objects;
- validates list backlinks, device/address owners and terminated interface labels;
- requires the complete count to equal the original kernel byte count;
- rewrites only when exactly one qualified layout matches and hidden addresses
  exist, using the same generated label matcher as the filled-buffer path.

Missing helpers, faults, unsupported layouts, mismatched counts and ambiguous
candidates preserve the original result. They do not become successful proof.
The hook-registration mask, admission priority and partial-health policy did not
change. The required behavioral test does not shrink to the installed mask.

## Independently derived layouts

Modern facts come from compiling [ifconf-offsets.c](../../kmod/test/abi/ifconf-offsets.c)
against the actual prepared reference headers and generated configuration. The
constant `.vpnhide_offsets` section contains nine little-endian u64s in source
order. The probe rejects randomized layouts, missing network namespaces, an
unexpected ifreq size and an unrecognized `sock_from_file` prototype. It does not
include the KPM consumer table. Compile with the ARM64 kernel include/generated
and UAPI paths, `-D__KERNEL__ -DMODULE -nostdinc`, plus the compiler resource
include directory; extract the section using `llvm-objcopy --dump-section`.
Exact commands, objects, section bytes, configs/image provenance and compiler
logs are retained in the build evidence.

The legacy container retains Images, not prepared sources. Its actual
`dev_ioctl`/`dev_ifconf`, `inet_gifconf` and `sock_from_file` instructions were
independently disassembled using their extracted kallsyms offsets. The device
list base/container subtraction, ip_ptr load, address-list/next loads, label
addition and second error-output argument establish the legacy entries below.
The existing socket offset is 32 for 4.x and 24 for the modern references;
sock_net is 48. Those socket offsets also have separate binding qualification.

| Reference | net.dev_base_head | dev_list | ip_ptr | ifa_list | ifa_next | ifa_label | sock_from_file args |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 4.4 | 272 | 80 | 696 | 16 | 16 | 72 | 2 |
| 4.9 | 288 | 80 | 720 | 16 | 16 | 72 | 2 |
| 4.14 | 272 | 80 | 704 | 16 | 16 | 72 | 2 |
| 4.19 | 272 | 80 | 696 | 16 | 16 | 76 | 2 |
| 5.4 | 144 | 80 | 720 | 16 | 16 | 76 | 2 |
| android12-5.10 | 144 | 72 | 720 | 16 | 16 | 76 | 2 |
| android13-5.10 | 144 | 72 | 728 | 16 | 16 | 76 | 2 |
| android13/14-5.15 | 144 | 64 | 744 | 16 | 16 | 76 | 1 |
| android14-6.1 | 144 | 64 | 720 | 16 | 16 | 76 | 1 |
| android15-6.6 | 144 | 64 | 728 | 16 | 16 | 76 | 1 |
| android16-6.12 | 144 | 352 | 968 | 16 | 16 | 76 | 1 |

The 5.10 difference is not inferred from the Android release at runtime. Both
reference candidates are tested against actual objects; ambiguous validation
refuses projection. These are reference layouts, not general vendor ABI proof.

## Final repeated matrix

All twelve profiles used the same final engineering KPM SHA256
`8a6a1c8fcdb42ee2a1d131fc8463f7a8011ceaf30b4bc323e197fadd41f251e2`
(101144 bytes) and guest tool
`bd7b64ae1263fa73a84f16e5cdd285abb4adbd6a6b431a6ee15ab7765fedf941`.
Each report has 36 accepted rows and two distinct RAW/protected boot IDs with
immutable plans. The new `ifconf_probe` row is **PASS on every profile**:
RAW `2/2/1`, protected `1/1/0`, zero stale VPN tail and preserved ordinary address.
Fresh NDK static-bionic getifaddrs/ioctl/bind probes were required, not skipped.

The final column separates PASS / native platform coverage / not applicable.
Legacy CAP_NET_RAW denial is measured platform coverage, not proof of our bind
hook; symmetric ENOPROTOOPT makes the old ifindex API not applicable. Those
existing classifications were not changed to obtain a successful report.

| Profile | Image SHA256 | PASS / native / N/A |
| --- | --- | --- |
| android12-5.10 | `38b08788eb8cbd3815fcdbcf38218701c1d46940a071a852f32801c1b9329b85` | 36 / 0 / 0 |
| android13-5.10 | `5ce0d01fc2fcbd53c42c59eecba4b2f2566d998cef57223d5e13826b7445363e` | 36 / 0 / 0 |
| android13-5.15 | `fb26780c34d637395685312521e7052aaab2c09bbf3a146ea111e967fa665b94` | 36 / 0 / 0 |
| android14-5.15 | `03244311709a4940290085b8ec435de97d2f738d1ee7fab63a00ef3cd35592d9` | 36 / 0 / 0 |
| android14-6.1 | `c0472bd87ccc82dfa5e7d0a18a0333258d39ca1d2b425b8536a754a232cdd3f0` | 36 / 0 / 0 |
| android15-6.6 | `4ddf38a6e127cb5f933c4ff8480289b32df3b28e88fa1b85de12a55134736b57` | 36 / 0 / 0 |
| android16-6.12 | `7c26662577e75f200fd19074851912315339a7a869e5b979de18325f52cf6cb9` | 36 / 0 / 0 |
| 4.4 | `34641c0e035f7aa5f19aaaef04947d67011b73487b817fe4d4c30e5b08d33524` | 32 / 3 / 1 |
| 4.9 | `73290732764e014d54179156225c6ac6beeac80cdcc7875c2f1db7d228e15982` | 32 / 3 / 1 |
| 4.14 | `db202bff0934420bcc33ef143ea971e693ebf8810339bfaaea36f549c05ad6f6` | 32 / 3 / 1 |
| 4.19 | `bea961fb945f39bc87f2a7fcc767521c382d57badbcae09e33a808e3d4224742` | 32 / 3 / 1 |
| 5.4 | `bfaa082198024a059cc1003817bdb91ca1a1ef4ee747cc2df65194262f2a433a` | 32 / 4 / 0 |

Legacy Images came from public OCI index
`4d34872312c23b159ee550fc2306b989652f44e170c6a1a2acadcd9cd33f30cf`,
amd64 manifest
`b08df9c77634961d8b82010f9b1f2f27c768237ec31d4545c843e901b2f1256d`.
Their AOSP source pins remain in
[build-source-kernel.sh](../../kmod/test/build-source-kernel.sh); per-profile
modern OCI manifests/build source pins and exact input hashes are retained.
These are private engineering inputs, not a rebuilt same-commit release bundle.

## Host checks and remaining gates

The host regression compiles the actual production walker against independent
golden memory fixtures for every table. It covers socket namespaces, aliases,
ordinary preservation, missing helpers, malformed ownership/backlinks/labels,
count changes, unaligned lengths, read faults, cycles, bounded work, unsupported
families and ambiguous candidates. Optimized GCC and ASan/UBSan runs pass. CI
runs this regression with a ten-second timeout. Thirty tooling tests and strict
all-target/all-feature tooling Clippy pass, including rejection of the original
size/fill discrepancy. C/Rust format and scaffold/no-su boundaries pass. The
startup slice was rechecked with five real-child admission/cancellation tests
and three IPC startup tests; whole-task final/device acceptance remains separate.

This is a Linux QEMU native harness qualification, including actual static-bionic
code, **not an Android Zygote/SELinux worker or genuine 32-bit compat-process
qualification**. It does not close the autonomous proc-interface leaks, remaining
route/rule/framework/old-process diagnostic coverage, provider lifecycle or the
paused backend-selection policy. Real device and final paired APK/Core acceptance
are still required.
