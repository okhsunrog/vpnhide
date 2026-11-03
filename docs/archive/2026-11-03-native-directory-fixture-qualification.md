# Controlled interface-directory qualification — 3 November 2026

Engineering slice after `069bd3669247f018831252d02aec54057cff8019`.
Exact changed-source hashes, actual ELF headers, test reports and QEMU input
identities are retained in the execution workspace under
`vpnhide-build-evidence/rfc2/native-directory-fixture/qualification.json`.
This is scoped Linux evidence, not Android device acceptance or a release.

## Implemented flow

The genuine Android collector's shared native observation now includes six
independent directory reads: libc `opendir/readdir` and direct Linux
`openat/getdents64`, each for `/sys/class/net`, `/proc/sys/net/ipv4/conf` and
`/proc/sys/net/ipv6/conf`. Paths are fixed; no arbitrary root path, process or
command was introduced. The direct reader parses the fixed getdents64 wire
instead of casting it to an architecture-dependent libc `dirent`. Both readers
bound records, names and time and close their descriptors.

The existing owned DOWN TUN supplies the positive stimulus. Core evaluates
negative, root start, worker, root end and cleanup controls. Root and same-worker
RAW qualifications intersect proof and retain the union of qualified leaks.
Missing/denied reads and lost ordinary `lo` visibility cannot prove concealment.
Later missing or malformed controls withhold proof without discarding a leak
already qualified by valid initial controls. Partial native reports survive
later framework failure through the existing coordinator path.

Only an enabled optional filesystem feature makes this group applicable.
`SysInterfaces` requires all six directory checks. The separate `Filesystem`
stat/access/open group remains unverified; this slice cannot complete the suite.
Typed FFI and exported retained evidence preserve actor, surface and controls.

## Real isolated execution

The reference Linux6.1 Image has SHA256
`c0472bd87ccc82dfa5e7d0a18a0333258d39ca1d2b425b8536a754a232cdd3f0`.
The previously qualified RAW-provider `.ko` from `fc6ad142` is reused unchanged,
SHA256 `8be313ad60f84a80a27ff9eec5c67d1a503a85cc28a362f9b5df9812c0c36b65`.
Its source/provenance is not relabeled as a fresh same-commit release payload.

- RAW boot session
  `7ec0a7ea18a1c37157ad3c2f37559d7e0f43aeea3a6440360132ffc85c4999ba`:
  no native backend, all six directory observations expose the exact fixture,
  ordinary and cleanup controls pass; no skipped vectors.
- Protected boot session
  `6fc15fc1e59603f64671bbb54b55586605ea5a69f2f2b5ac1f644532dc800b8a`:
  all six checks hide the fixture and retain `lo`. The same-thread RAW interval
  observes all six; a concurrent thread of the same UID remains protected;
  confirmed revoke restores concealment. FD-close, expiry, duplicate-FD and
  crash cleanup controls and unchanged route observations still pass.

The existing four `/proc/net/dev` and `/proc/net/if_inet6` leaks remain actual
observed leaks in both reader actors. An oracle PASS means the test detected its
expected reference behavior, not that those product leaks disappeared.

## Compatibility and limits

Paired APK/Core RPC29 and diagnostic contract8 carry the new fields; debug
schema9 retains `run.sysInterfaces` and corresponding RAW observations. Private
journal13 and kernel control2/telemetry1 are unchanged. Version numbers alone do
not require rebuilding a user's builtin kernel. A provider without the optional
RAW capability remains usable, but cannot borrow positive app RAW proof.

Both Android FFI ABIs are built and their actual ELF LOAD alignment/congruence
checked at16KiB. Compilation does not establish execution of a secondary32-bit
Android worker. QEMU uses a static Linux collector and a UID-switched child;
neither is a genuine Zygote/bionic/SELinux acceptance substitute.

Actual Android directory permissions, both runtime ABIs, builtin execution,
KPM/Zygisk RAW transports, stat/access/open, routes/rules and remaining framework
controls still need code or separate device acceptance. Backend policy remains
paused. No phone, production signing, release or provider configuration was used.
