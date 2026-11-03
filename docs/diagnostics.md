# Autonomous diagnostics

Core owns diagnostic admission, required plans, fingerprint/cache, attribution,
assessment and cleanup. The current RPC contract publishes the retained run and assessment atomically.
Dashboard, Diagnostics, agent state and exports consume that same publication.
Opening the app, changing tabs, reading state or capturing logs does not run a
suite. Explicit Retry requests a run; repeated pending taps coalesce in the app
process, and RPC replies never become a competing result source.

A foreign VPN and the APK's membership in its split-tunnel list are irrelevant to
admission and Dashboard state. The former app-VPN root probe, periodic framework
classification, route/rule listener, Kotlin gate and main-process JNI suite are
removed. Real Connectivity APIs inside the diagnostic worker, system-server
fixtures, production hooks and explicitly requested manual forensics remain.

The former uncontrolled Rust `run_all_json`/`CheckStatus` suite, observation-v1
JSON wrapper and manual `probe checks` entry are also removed. No APK, export or
current RPC consumer required that root-only fallback. Collectors now return
bounded typed observations; only the shared diagnostic model evaluates evidence.
The generated interface vocabulary and independent malformed-netlink/readback
controls remain. An obsolete CLI request is refused before daemon/socket setup.

## Evidence and ownership

The required plan comes from the supported contract and enabled features, not
from the installed hook mask. Diagnostic contract9 requires SysInterfaces and
Filesystem only when the desired `filesystem_iface_paths` feature is enabled,
matching RFC0002's optional-filesystem scope. Default route/proc-interface and
framework requirements remain. Previous or truncated plans cannot be reused as
current proof; the plan is not narrowed to successfully observed stimuli. Every
required vector must have positive RAW stimulus, a protected observation, attribution and ordinary-object preservation
controls. Empty enumerations and clean Java replies alone cannot prove hiding.
A leak remains evidence under incomplete controls, worker death or cleanup failure.
After a vector's negative/start controls qualify the worker's observed leak,
an unavailable or malformed end/cleanup read, or a missing later component,
cannot erase it.
Those failures are recorded independently and withhold successful proof. Initial
RAW/worker data must still be available and valid; missing stimulus is not a leak.

The real `:diagnostics` Android worker is a Zygote-created app process. Core
orchestrates its bounded execution; Kotlin reports bounded observations rather
than declaring PASS. Native fixtures are owned TUN resources with session/boot/
identity and deadline cleanup. Core run admission, worker phases, reports and
completion use CLOCK_BOOTTIME, including time spent suspended. Active Condvar
waits recheck this clock at most every 250 ms of awake time so resuming does not
restart a full monotonic timeout. An expired run cleans its owned fixtures and
retains accepted observations; no timer claims cleanup can execute while the CPU
is asleep. Clock failures deny further worker authority. Framework envelopes inherit the original run expiration; beginning Java collection never renews it. Framework fixtures exercise actual public APIs,
Binder, source transformations and production normalization. Framework RAW grants
are scoped to the exact test process and never turn off protection for the whole UID.
Native collection now uses one dedicated Android collector thread. Core authenticates
its genuine Zygote process and revalidates thread membership/birth plus PID, user
and network namespaces at each phase. Root controls and same-worker RAW/protected
observations are evaluated together; root visibility alone cannot prove the app's
RAW visibility. An absent/unsupported grant withholds native proof while retaining
qualified protected leaks. Unknown activation/revocation never authorizes a
protected sample. No classifier or grant decision lives in Kotlin.

The `.ko` and builtin now have a private diagnostic-thread RAW provider (wire1).
Root arms an expected UID/TID and nonce; the actual collector self-registers its
retained kernel PID identity through an ordinary loopback `SIOCGIFNAME`, then
root activates that same identity. Registration alone grants no RAW. Other
threads of the same UID remain protected. The fixed five-second CLOCK_BOOTTIME
lease cannot be renewed by replay; matching revoke, owner FD release and expiry
restore protection. The separate root-only proc endpoint and ioctl handshake do
not change control2/telemetry1 or require broader app SELinux proc access.
Absent kernel helpers leave this optional capability unavailable without changing
backend health or triggering fallback.

The Root lease/controller and unprivileged collector adapter have Linux fixture
oracle coverage for real same-thread RAW/protected results, same-UID thread
isolation, ordinary loopback preservation, final owner close and TTL expiry.
The Android worker handshake and assessment consumers are implemented and have
host/JVM/cross-build coverage; genuine Android bionic/SELinux/background execution
still needs device acceptance. KPM and Zygisk RAW provider adapters remain code
work. A second-ABI worker launcher also remains code work: an ordinary
Service uses the package's selected ABI. AOSP's `instrument --abi` force-stops the
target package before creating its requested process; `--no-restart` only attaches
to an existing main process. Neither is a safe background launcher for this APK.
See the [source review](archive/2026-11-03-diagnostic-worker-abi-launch.md). Builtin source export/compilation is separate from a builtin boot; Linux
`.ko` evidence cannot be borrowed as builtin or Android acceptance.

Native observations are delivered to Core **before** framework collection, with a
separate worker-completion ACK. Java failure, worker death or a later deadline
cannot erase an already accepted native observation. Core retains scope receipts
and failed RAW samples separately, and emits the same assessment to every read
consumer. The old root-only native finish entry point exists only in the explicit
fixture-oracle build, not as a production fallback. Opening the app still never
admits a suite.

Required/proven/leaked/unverified, infrastructure failures and cleanup are separate.
A worker's fresh result does not prove that already-running games consumed a new
Zygisk generation. Cache identity includes policy, runtime/platform evidence and
loaded-generation coverage. Missing required proof remains unverified. The
current partial implementation has no full-success assessment variant.

Core prepares its managed diagnostic target during production boot, after PM
readiness and before backend policy is frozen, without waiting for the APK to
open. Preparation uses the durable writer and source-checked legacy bootstrap;
it does not delete old sources. An incomplete/malformed source cannot be silently
replaced by empty config. The APK startup adapter may request the same fixed
`PrepareSelfTargets` command for recovery; it supplies no replacement config and
queries the same operation after an uncertain reply. Diagnostic admission remains
blocked until managed target intent is present. This does not prove the LSPosed
generation or previously running app processes have consumed it.

## Implemented scope and remaining implementation

The base required plan has eighteen vectors. Native interface-only TUN stimuli
and attributed framework capability/link plus two legacy NetworkInfo scenarios
have executable cloud coverage. Legacy TYPE_VPN and enumeration require separate
controls and per-invocation CLOCK_BOOTTIME windows; ordinary duplicates and each
real invocation's baseline must be preserved. Prepared and revoked controls check
fixture absence rather than absence of the user's actual networks.

Independent libc and direct-syscall socket binding have root differential, exact
readback and ordinary loopback controls. Permission errors and missing readback
remain unverified; a setter error cannot erase an observed binding leak. The
worker must share the fixture network namespace.

Address observations distinguish libc from direct syscall ioctl/netlink actors.
The owned DOWN TUN receives exclusive benchmark IPv4 /32 and private IPv6 /128
addresses with NOPREFIXROUTE (and IPv6 NODAD); setup never raises the link or
changes ordinary interfaces. Address availability is verified independently for
each family. Unsupported/failed setup leaves the corresponding checks unverified.
SIOCGIFCONF NULL-buffer size and data are separate checks, qualified by complete
stable RAW lists and the worker's actual ifreq size. Interface-only getifaddrs
proof cannot establish IPv4/IPv6 coverage. Core requires all eighteen address
subchecks before reporting complete scoped address coverage; this is not a full
suite PASS. Cleanup and ordinary loopback controls remain mandatory.

The `ProcInterfaces` group has four required subchecks: `/proc/net/dev` and
`/proc/net/if_inet6`, each through libc and direct openat/read syscalls. Readers
parse bounded real records; missing or denied files do not prove hiding. Exact
owned names/IPv6 addresses are attributed through negative/start/worker/end/
cleanup observations and loopback controls. Traffic counters are not identity
and need not remain constant. The current `.ko` QEMU test positively observes
all four leaks, even though address ioctls/netlink hide successfully. These are
retained leaks, not a missing-stimulus result or an automatic fallback trigger.
This group does not qualify proc routes, sysfs/proc-sys, TCP sockets or Ports.

The optional `SysInterfaces` group independently enumerates `/sys/class/net`,
`/proc/sys/net/ipv4/conf` and `/proc/sys/net/ipv6/conf` through libc
`opendir/readdir` and direct `openat/getdents64`. The existing owned DOWN TUN
supplies the positive names; ordinary `lo` must remain visible. Core requires
all six negative/RAW/protected/end/cleanup controls, including same-worker RAW,
before reporting scoped directory coverage. Missing IPv6, denied sysfs/proc-sys,
empty lists and lost cleanup remain unverified; qualified leaks survive late
failures. This evidence is ignored when the optional feature is not requested.
The separate optional `Filesystem` group now observes eighteen path-lookup
checks: libc and direct syscalls × these three parent directories × stat,
access and open-directory. Each exact owned fixture path needs negative, positive
RAW, protected, end and cleanup controls; ordinary `lo` paths stay visible.
A successful lookup reveals the path. Only `ENOENT` means absence; denial,
unsupported syscall or missing positive stimulus cannot prove hiding. Qualified
worker leaks survive failed ordinary/end/cleanup controls, which independently
withhold successful proof. Root and same-worker RAW proof intersect as for the
other native groups. These lookups cannot substitute for directory enumeration.

Direct stat uses Linux statx's fixed 256-byte UAPI output buffer, without guessed
ARM32 stat/stat64 layouts or private offsets. Older kernels that return `ENOSYS`
remain unverified. Actual bionic, SELinux and secondary32-bit Android worker
execution remain separate gates; both compiled ABIs alone establish neither.
See [the scoped Linux qualification](archive/2026-11-03-native-filesystem-fixture-qualification.md).

An [engineering owned-UP route prototype](archive/2026-11-03-native-route-prototype.md)
now positively qualifies eight libc/syscall netlink/proc IPv4/IPv6 vectors on the
reference Linux6.1 with RAW/protected, ordinary and lifecycle controls. It preserves
the primary DOWN fixture and is gated out of Core/APK production builds. Its
resource admission/recovery, genuine-worker report and FFI/export wiring remain
code work; current production route checks remain unverified. No required check
was removed or inferred from installed hooks.

Rules, remaining route forms, proc/socket stimuli and framework active/
default/callback/PendingIntent controls still need implementation or expansion.
Android bionic, SELinux, Java callback isolation and old game-process coverage need
real device acceptance. QEMU/static libc evidence does not replace those gates.
The [roadmap](../ROADMAP.md) separates these
open code tasks from device/signing/publication requirements. See
[RFC2](rfcs/0002-autonomous-diagnostics.md) for the full contract.

## Worker ABI coverage

The genuine `:diagnostics` Service currently starts with the APK's selected ABI.
On a mixed-ABI device, it does **not** independently request a Zygote32 worker.
A safe secondary32-bit launcher is still unimplemented code, in addition to the
later device acceptance. ARMv7 compilation, ELF qualification, a static Linux
compat process or a successful ARM64 worker does not fill that gap. ABI-specific
loader image proof remains required; the implementation does not borrow one
ABI's evidence for the other or turn an unavailable worker into PASS.

The reviewed `am instrument --abi` candidate was rejected because its target
package restart would interrupt the running APK and unsaved drafts. Root
`app_process` with setuid is not a genuine worker substitute. No helper-APK install
flow is assumed or added. See the [launcher source review](archive/2026-11-03-diagnostic-worker-abi-launch.md).

## Read models and captures

`AutonomousDiagnosticSnapshotOwner` combines accepted Core/config/root identities
without RPC admission. Canonical equality, config revision and Core epoch qualify
diagnostic config acceptance; unrelated publication sequence changes do not
withdraw an unchanged configuration. Heavy root metadata has its own source join.
Retained evidence is visible with its currentness, connection and source identity.

Core's native read model separately reports selected implementation/provenance,
attempts, delivery, observed runtime, required/installed/missing hooks, setup and
filesystem state. Setup readiness is not a diagnostic result. A receipt whose
preparation failed cannot reuse an older delivery at the same revision. Observed
runtime alone cannot borrow an old bundled payload hash or global Zygisk coverage.

Schema8 exports both read models, per-actor address/proc evidence, the full retained RAW receipt and source identities. Historical gate/report/
verdict fields are null; their old Kotlin domain owners are removed. Capture time
is not measurement time. Manual `NetworkViewCapture`, hook counter baselines,
unsigned delta/reset handling and root forensic sections remain available for
investigation. See [debug bundles](debug-bundle.md).
Numeric counter differences have one owner in `protocol::telemetry::counter_change`.
The UI/agent capture uses a bounded local FFI batch per backend document; forensic text renders the
same typed Unavailable/Reset/Count result. A captured empty baseline admits new
rows, while an absent baseline remains unavailable. Counts retain all unsigned
64 bits: crossing the signed `Long` boundary is not a reset, and a decrease is
never guessed to be wraparound traffic. A discontinuity requests re-baselining;
it does not prove a particular process/backend restart or authorize fallback.
Snapshot generation/source attribution and remaining capture/export orchestration
are separate work; this numeric comparison does not certify either.
