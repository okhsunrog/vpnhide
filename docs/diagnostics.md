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

## Evidence and ownership

The required plan comes from the supported contract and enabled features, not
from the installed hook mask. Diagnostic contract6 requires SysInterfaces and
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
identity and deadline cleanup. Framework fixtures exercise actual public APIs,
Binder, source transformations and production normalization. Framework RAW grants
are scoped to the exact test process and never turn off protection for the whole UID. Native fixtures currently use privileged root
negative/start/end/cleanup controls and one protected app-worker sample. A native
RAW grant for that exact worker process remains code work; root controls do not
establish its same-process RAW visibility or complete native attribution. Current
scoped native results and their qualifications must not be presented as a full
suite PASS. Adding more collectors does not close this missing grant mechanism.

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

The base required plan has nineteen vectors. Native interface-only TUN stimuli
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

Routes/rules, remaining proc/sys stimuli and framework active/
default/callback/PendingIntent controls still need implementation or expansion.
Android bionic, SELinux, Java callback isolation and old game-process coverage need
real device acceptance. QEMU/static libc evidence does not replace those gates.
The [roadmap](../ROADMAP.md) separates these
open code tasks from device/signing/publication requirements. See
[RFC2](rfcs/0002-autonomous-diagnostics.md) for the full contract.

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

Schema7 exports both read models, per-actor address/proc evidence and source identities. Historical gate/report/
verdict fields are null; their old Kotlin domain owners are removed. Capture time
is not measurement time. Manual `NetworkViewCapture`, hook counter baselines,
unsigned delta/reset handling and root forensic sections remain available for
investigation. See [debug bundles](debug-bundle.md).
