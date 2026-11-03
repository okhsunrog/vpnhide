# Owned UP route prototype — 3 November 2026

Engineering slice after `14c2f01a160632dfb903676ba9049408569b712b`.
This is a Linux/reference-kernel prototype, **not production autonomous route
coverage** or Android acceptance. Source hashes, binaries, serial logs and final
checks are retained under `vpnhide-build-evidence/rfc2/native-route-prototype/`.

The existing TunFixture/NativeFixtureSpec remains DOWN. An explicit engineering
consuming transition returns a distinct UP owner, so the old DOWN assessment
cannot be called on that object. A separate pair uses a VPN-pattern name and a
non-VPN ordinary name. Both nonpersistent TUN owners carry exclusively created
IPv4/IPv6 host routes to reserved benchmark/private destinations. No default route,
policy rule, persistent interface, packet-reader loop or delete-by-name is added.
Closing the last descriptors owns address/route/interface removal.

One portable Rust model describes literal routes and evaluates controlled evidence.
The read adapter collects libc/direct-syscall RTM_GETROUTE and literal
`/proc/net/route`/`ipv6_route` separately: eight actor/family/surface vectors.
Proc names are not resolved through another potentially hooked interface API.
Native-endian IPv4 proc words, network-byte IPv6 text, route masks, family/attribute
widths and framing are checked. The closed proc reader is shared with the existing
interface adapter, retiring its duplicated implementation. The common netlink
framer preserves valid prefix frames for read-only consumers while full mutation
ACK validation still rejects any malformed suffix.

Already observed valid rows survive later read, UTF-8, parse, frame or dump errors.
Only qualified controls can establish a leak; unavailable RAW is missing positive
stimulus. Destination, interface index and interface name are independent exposure
tags. Clearing destination alone cannot prove hiding. Root negative/start, actual
worker RAW/protected, Root end, post-close cleanup and ordinary-object preservation
remain separate. Late ordinary/end/cleanup failures keep qualified leaks and
withhold proof. No cleaned answer, synthetic end or pre-close cleanup is supplied
to the running oracle evaluator.

The final reference Linux 6.1 boots have no skips:

- RAW session `1e310bca6b22c609244a933a31d7ccf30f170fe62c769d0f7ea6890d5a0795c7`
  positively observed all eight vectors without a backend.
- Protected session `b51481ad1e45a43a2e257449d4c11601b0b7815b8ef94995933ec8aabc026ae0`
  proved these eight scoped vectors with ordinary routes preserved. RAW on the measured thread
  and protection on another thread of the same UID were independently sampled.
- Duplicate-FD retention, last-close removal, actual `_exit` process cleanup and
  an injected failure after the first acknowledged route all passed independent
  readback. The preexisting native fixture controls remain and the four known
  proc-interface leaks are still recorded; they were not fixed or hidden here.

The reference Image hash is
`c0472bd87ccc82dfa5e7d0a18a0333258d39ca1d2b425b8536a754a232cdd3f0`.
The `.ko` is the existing `fc6ad14253ce4a866675f2f91a950e9bfc574c9b`
provider, SHA256
`8be313ad60f84a80a27ff9eec5c67d1a503a85cc28a362f9b5df9812c0c36b65`;
it is not restamped as a payload from this source snapshot. Loader preflight and
17-hook registration passed in the protected VM. The musl oracle linker emits its
existing clang `-no-pie` unused-argument warning; Rust Clippy remains strict.

Final integration checks: 651 Rust tests and 587 JVM tests, zero failures/skips;
strict host/all-target Clippy and both Android ARM library targets including the
oracle helper, fmt/ktlint/detekt/CPD/lintDebug, scaffold/no-su and actual FFI 16 KiB
LOAD alignment/congruence. These are compile/behavior gates, not Android runtime
acceptance.

Production work remains: admit/retain the additional owned resources in the Core
run, carry observations through the genuine worker/FFI/export path, synchronize
contracts and wire bounds, and extend fingerprints/cleanup recovery. Rules,
additional route forms and framework controls remain separate. The required plan
is not narrowed, and reads still cannot admit a suite. RPC31/journal14/diagnostic9/
debug schema11 are unchanged because these new route types are not reachable from
the current production worker/report wire. The prototype and its mutating UP
owner are gated by `fixture-oracle`, which is not packaged in Core or APK.

This does not qualify Android bionic/SELinux, a genuine secondary 32-bit Zygote
worker, KPM/Zygisk RAW transport, arbitrary vendor kernels or old game-process hook
generations. Those are distinct code and acceptance items. No phone, production
key, release, provider setting or boot write was used; backend policy stays paused.

## Transport follow-up

The worker RPC frame limit is 1 MiB; full routing tables are not appended to the
production native packet. The engineering route worker now accepts the issued
spec and returns original matching records, retaining all three exposure tags and
any earlier read error. This is report projection after actual API reads, not a
replacement network response. At most sixteen matching rows per vector reserve
less than 128 KiB for this component, verified with maximum-sized valid fields.
Overflow remains explicit with retained rows, so it cannot prove absence. The
full production native-report/publication budget and route wire/resource admission
still need integration.

The projected-report RAW/protected rerun retains the same eight-vector and lifecycle
results, without skips: RAW `f8db8f9ebbe6c648b0465e73ee6643cdd6047d133c251f256610cb89d39e6228`,
protected `16306503e5c511ca345ed4ac103988f457ea690ab7e45c53b5beb7dfd98a4022`.
Evidence is retained in `vpnhide-build-evidence/rfc2/native-route-scope/` separately
from the earlier full-table prototype. This follow-up changes only the local model
and engineering worker; production RPC/schema versions and its missing-route status
remain unchanged.

## Cleanup ownership follow-up

The engineering owner now captures a known boot UUID and the current thread's
network-namespace identity before allocation. Cleanup retains actual ifindices and
both names around the ordinary-interface rename. Detailed setup failures retain
created/unknown ownership plus independent readback, rather than discarding that
information into an error string. Unknown identities and namespace/boot changes
withhold cleanup; the code never deletes a possibly replaced object by name.

Three targeted ownership/request tests, strict host/all-target and both Android
ARM library Clippy checks passed. RAW `52d7291cc44cf9863e5152fb700dcf499b8d6c893d296979cac8402c223f42d8`
and protected `6561f3d6243be53941e8d1ca301e3267330c3183d06fa2a295ab995506c4014a`
passed all eight route vectors and lifecycle controls without skips. Evidence is
retained separately under `vpnhide-build-evidence/rfc2/native-route-ownership/`.
The run-coordinator/worker wire is still not connected; this remains a gated
engineering fixture. No new backend-policy implementation, phone change or public
release is included in this ownership correction.
