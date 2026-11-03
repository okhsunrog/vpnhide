# Controlled interface-path lookup qualification — 3 November 2026

Engineering slice after `7f95e67257ce3f0143f5360b6224b5af6383039b`.
Exact source hashes, real test logs, ELF headers and QEMU input identities are
retained under `vpnhide-build-evidence/rfc2/native-filesystem-fixture/qualification.json`
in the execution workspace. This is scoped Linux evidence, not a release or
Android device acceptance.

## Implemented flow

The Android worker's shared collector observes eighteen independent path checks:
libc and direct syscalls, each performing stat, access and directory-open against
the exact owned TUN name under `/sys/class/net`, `/proc/sys/net/ipv4/conf` and
`/proc/sys/net/ipv6/conf`. Parents are a closed enum; the generated name is
validated. No arbitrary root path or command is accepted. Opened descriptors are
owned and closed, with CLOEXEC.

Direct stat uses Linux statx's fixed 256-byte UAPI buffer, without guessing
ARM32 stat/stat64 layouts or reading private offsets. A successful lookup exposes
the path; only `ENOENT` establishes absence. Permission failures and unsupported
syscalls retain their errno. Older kernels without statx remain unverified for
that required actor.

Core evaluates negative, positive, worker, end and cleanup controls, preserving
ordinary loopback paths. Root and same-worker RAW proof intersect; qualified
leaks form a union. Valid initial controls can qualify a leak even when ordinary,
end or cleanup observations later fail; those failures independently withhold
proof. Missing initial stimulus cannot qualify a leak. Matrix regressions cover
all eighteen cases, missing/denied/unsupported reads, malformed/duplicate
measurements and late failures.

The optional `Filesystem` group requires all eighteen checks only when desired
`filesystem_iface_paths` is enabled. Six directory-enumeration checks remain the
separate `SysInterfaces` requirement. Neither substitutes for the other. Actual
native results reach Core before framework collection and survive later worker
failure; typed FFI and exports retain actor, surface and operation.

## Real isolated execution

The reference Linux6.1 Image is unchanged, SHA256
`c0472bd87ccc82dfa5e7d0a18a0333258d39ca1d2b425b8536a754a232cdd3f0`.
The RAW-provider `.ko` from `fc6ad142` is unchanged, SHA256
`8be313ad60f84a80a27ff9eec5c67d1a503a85cc28a362f9b5df9812c0c36b65`.
Its provenance is not relabeled as a new same-commit payload.

- RAW session
  `e3fe16e8ebd68cce6cae06dbd559aafe063d647a80487d296b6b0131e5bbedc9`:
  all eighteen paths visible, ordinary and cleanup controls pass, no skipped
  vectors and no loaded backend.
- Protected session
  `5fad8474a68e1d0eec15218144d31675553716d930ce1329f06dfdb90d5bbf21`:
  all eighteen fixture paths hidden, ordinary paths preserved. Same-thread RAW
  sees all eighteen while a concurrent thread of the same UID stays protected.
  Confirmed revoke restores concealment. Existing FD-close, expiry, duplicate-FD,
  crash cleanup and route-preservation controls still pass.

Four previously observed proc-interface leaks remain. Oracle PASS means its
expected observations and controls passed, not a whole-suite PASS.

## Compatibility and remaining work

Paired APK/Core RPC30, diagnostic contract9 and debug schema10 carry the fields.
Private journal13 and kernel control2/telemetry1 are unchanged. A version number
alone does not require rebuilding a user's builtin kernel. Missing optional RAW
capability withholds proof without changing backend policy.

Both actual Android FFI ABIs are built and checked for16KiB LOAD alignment and
offset/address congruence. These are compile/ELF gates, not runtime coverage.
**A genuine secondary32-bit Android worker on a mixed-ABI device remains an open
implementation gap.** The normal Service uses the primary ABI; instrumentation
was rejected because it force-stops the APK. Primary64-bit evidence cannot prove
32-bit execution; loader image-proof gates remain unchanged.

Actual bionic/SELinux permissions and suspend/cleanup, builtin execution,
KPM/Zygisk RAW transports, routes/rules, remaining proc/socket and framework
callback/PendingIntent controls require further code or device acceptance as
listed in the roadmap. QEMU static libc/UID-switched Linux children do not replace
Zygote workers. Backend policy remains paused. No phone, provider configuration,
production signing, release or boot write was used.
