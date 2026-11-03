# KPM diagnostic RAW lifecycle review — 3 November 2026

Historical source review after `8bc47786`. **No KPM RAW provider or controller
from this experiment is enabled or integrated in main.** Current remaining work
belongs to [ROADMAP](../../ROADMAP.md).

The `.ko`/builtin provider has an owned private proc descriptor. KPM deliberately
has no proc mirror: adding one would require another private proc/file-operation
ABI. An offline prototype instead reused its authenticated `ctl0` transport,
private capability/query framing, the existing exact-thread PID-reference state
machine and an input-only loopback ioctl registration. Strict freestanding ARM64
compilation succeeded; this is neither execution nor lifecycle qualification.

Inspection of vendored KernelPatch
[`0ceeeb968bef86b48af68640af8b135215dc3399`](https://github.com/bmax121/KernelPatch/tree/0ceeeb968bef86b48af68640af8b135215dc3399)
identified a concrete unresolved boundary in
[`kernel/patch/module/module.c`](https://github.com/bmax121/KernelPatch/blob/0ceeeb968bef86b48af68640af8b135215dc3399/kernel/patch/module/module.c#L456):

- `unload_module` removes the module and frees its executable, arguments and
  object after calling `exit`, even when `exit` returns an error. Returning
  `EBUSY` from the module does **not** keep it resident.
- `module_control0` replaces a module-owned argument buffer and invokes its
  callback inside a reader section. Unload does not exclude/wait for those live
  calls; concurrent controls can also replace that same buffer. Core's shared
  userspace KPM lock coordinates its own callers, not every possible runtime caller.

An exact retained PID reference plus a five-second TTL does not by itself solve
module-code/control lifetime. A safe KPM adapter must establish this runtime
boundary and eventual reference cleanup, including control/unload contention,
before production grants. Do not fix it with a spin loop, guessed lock layout,
numeric PID substitution, ignored refusal or a claim that exit status pins code.
This finding is pinned to the reviewed source; other actual APatch/Next/fork
runtimes require their own evidence.

The disabled prototype patch, source hashes, strict compile log and quarantined
object are retained in
`/workspace/vpnhide-build-evidence/rfc2/kpm-raw-experiment/`. Its patch SHA256 is
`2c3af44516131d3438b7552ceb416ead532702fdc6375dcba6d50042a31de4ca`.
All prototype production-source edits were restored and the ordinary KPM producer
was rebuilt, so later assembly cannot accidentally consume the experimental object.

Remaining **code/qualification**: runtime lifetime correction or a verified safe
adapter contract, Core KPM RAW transport, controlled execution and crash/unload
regressions. This is not merely Android device acceptance. The separate approved
provisioner/fork work may carry a reviewed correction; no fork target, publication,
phone operation or boot write was chosen here. Paused backend policy is unchanged.
