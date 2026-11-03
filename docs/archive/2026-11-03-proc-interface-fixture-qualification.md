# Proc interface fixture observations — 2026-11-03

Engineering evidence for RPC26 / diagnostic-contract5 / debug schema7. Journal12
and kernel control2/telemetry1 are unchanged. This adds observation and Core
assessment, not new backend filtering or selection policy.

## Real execution

The owned DOWN TUN and its addresses provide positive stimuli for
`/proc/net/dev` and `/proc/net/if_inet6`. The shared collector reads both files
through independent libc and direct openat/read syscall actors. Core evaluates
four required subchecks against negative/start/worker/end/cleanup observations,
including ordinary loopback controls. Empty or denied reads are unverified.
Traffic counters are parsed for validity but are not stable object identities.

| Actual QEMU mode | Address subchecks | Proc interface subchecks |
| --- | --- | --- |
| RAW, no backend | 18 leaks, zero control failures | 4 leaks, zero control failures |
| Current matching `.ko` | 18 proven, zero control failures | **4 leaks**, zero control failures |

The `.ko` genuinely leaves these proc interfaces visible in this Linux guest.
The oracle's PASS means controlled **leak detection and cleanup** worked; it does
not mean proc concealment, full protection or the complete suite passed. The
Core grouped assessment retains `ProcInterfaces` as leaked. Installed address
hooks, otherwise proven address vectors, permission denials and empty lists
cannot turn that result into proof. No automatic fallback is triggered.

Both runs also passed normal cleanup, retained-last-FD detection, process exit,
post-allocation setup failure and post-IPv4/pre-IPv6 failure. Partial address
setup preserved its two device-name leaks while leaving IPv6 subchecks
unverified. Closing the final FD removed the fixture and restored the original
address/route controls. The protected boot also passed preflight before loading.

## Identities and evidence

The kernel, matching engineering `.ko` and guest executor are unchanged from the
[address qualification](2026-11-03-native-address-fixture-qualification.md).
These are isolated engineering inputs, not a newly assembled same-commit release.

- Core oracle: 797440 bytes, SHA-256
  `cd5b408d432c3b7afa1c2282943ea2db1ed9a397dcb1c0f8074266c0277ad505`.
- RAW session:
  `c635c1c030aef954cf9e6761f22431830d5a1d7519f69cd943e0e1ab0f9a0cf1`.
- Protected session:
  `4b42c4906b70a3a76c87f07c0dabd9fc65f7f754efb57bbcc9abb5b36c391206`.

Retained cloud logs, input identities, typed plans and build/test logs are under
`/workspace/vpnhide-build-evidence/rfc2/proc-interfaces`. The typed QEMU runner and
explicit oracle boot marker remain the only test entry path.

Validation: 611 selected Rust tests and 575 JVM tests passed without failures or
skips; strict host and both Android ARM Clippy, both real APK native ABI builds,
Android lint, ktlint/detekt/CPD, formatting and module/no-su boundaries passed. Adversarial
tests cover absent RAW positives, read denial, empty observations, missing
actors, malformed/duplicate records, scrubbed identity and failed cleanup.

## Remaining code and acceptance

Kernel proc-file concealment remains code work; this slice does not repair that
gap or weaken the required plan. KPM NULL-buffer ifconf filtering also remains.
Routes/rules, sysfs/proc-sys, sockets/Ports and remaining framework controls are
separate unfinished scopes. The grouped proc result qualifies only the two
declared files through both actors.

This setuid Linux oracle does not qualify Android Zygote/bionic/SELinux behavior,
mixed worker ABIs, Java, Zygisk, old app processes or ordinary user traffic.
No device, production key, release, provider or boot-write operation was used.
See the [current contract](../diagnostics.md) and [roadmap](../../ROADMAP.md).
