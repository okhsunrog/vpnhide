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
from the installed hook mask. Diagnostic contract11 requires SysInterfaces and
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
is asleep. Clock failures deny further worker authority. The run allows up to
30 seconds to prepare and authenticate a fresh worker. The first accepted
worker claim starts a 10-second native measurement budget. Acceptance of that
report starts a 25-second framework budget. Each stage remains within 65
seconds of admission. Replayed requests cannot renew a deadline. The framework
envelope inherits the fixed framework deadline, and the native RAW grant keeps
its independent five-second limit. Framework fixtures exercise actual public APIs,
Binder, source transformations and production normalization. Framework RAW grants
are scoped to the exact test process and never turn off protection for the whole UID.
With Debug logging enabled, Core logs boot-clock timestamps for preparation,
launch, framework RPCs and cleanup. Debug APKs also time the worker's connection,
native measurement and each Java probe group.
Native collection uses one dedicated Android collector thread. Core authenticates
its genuine Zygote process and revalidates thread membership, both birth identities
and the shared network namespace at each phase. PID and user namespaces are also
checked when the kernel exposes them. On kernels that omit these namespaces, the
corresponding procfs entries must be absent for Core, the worker and its collector.
Root controls and same-worker RAW/protected
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
The Linux adapters compile shared atomics inline, without compiler-rt helpers.
The `.ko` calls its dynamically resolved PID/clock helpers through narrow,
non-inlined `__nocfi` boundaries: kallsyms addresses are not canonical jump-table
entries on older CFI kernels. Other module callsites keep their CFI checks.

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

With Debug logging enabled, Core logs the first failed step of each admitted
framework run. The line includes the run ID, requested phase, active phase,
number of accepted samples, and failing operation. The assessment and IPC
contracts stay the same.

`system_server` publishes each phase acknowledgement by atomic rename. If
the file is replaced while Core reads it, Core waits for a stable read under
the existing phase and run deadlines. Only the exact current session and
phase can advance. Invalid ownership, malformed data and in-place mutation
still fail the read.

The framework channel closes every descriptor it opens with `Os.open`.
Android streams constructed from a supplied descriptor borrow it, so the
channel closes the authority input and acknowledgement output explicitly on
success and failure. During Pixel stress testing of the previous build,
four runs left 2,112 authority descriptors and 136 acknowledgement
descriptors open in `system_server`. A longer series exhausted the
system service's file descriptor limit. Core timed out waiting for the
Protected acknowledgement shortly before Android logged `EMFILE` and
restarted `system_server`.
The fixed APK passed 64 consecutive runs on the unlocked Pixel with the same
`system_server` process. Each run confirmed 13 groups with no observed leaks
or framework errors. Neither channel file had a remaining descriptor after
any run.

On the Pixel 8 Pro, the previous Core lost a `Revoked` acknowledgement in
one of ten runs. With the fix, all ten runs retained all five scoped framework
results and Core removed the fixture authority. The overall diagnostic plan
remained at twelve confirmed groups out of twenty.

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

Core admits an independent pair of UP interfaces for the eight libc/direct
netlink and proc IPv4/IPv6 route vectors. The [Linux prototype and qualification
history](archive/2026-11-03-native-route-prototype.md) documents the fixtures.
The primary interface fixture stays DOWN. The route pair uses exclusive host
routes and closes both nonpersistent TUN descriptors before independent route
and interface cleanup readback. Unconfirmed cleanup blocks subsequent run
admission until verified. A failed setup with confirmed cleanup leaves its
route groups unverified and permits the other vectors to run.

Core creates the route pair before measuring the primary fixture's baseline
and keeps it through the primary cleanup sample. This keeps the two route
interfaces from changing the population measured by `SIOCGIFCONF`.
Worker-side link enumeration, flags, MTU and socket binding qualify their
RAW and protected controls independently. A denied `RTM_GETLINK` dump
leaves that check unverified while the other observations can qualify.

The worker measures routes on the existing RAW/protected collector thread
under one second of read budget per route observation. Its report contains
original rows attributable to the issued fixture, with at most sixteen rows
per vector. Core rejects route rows outside the issued fixture and evaluates
netlink and proc groups separately, each requiring both actors and both
families for proof. Errors and late cleanup failures retain qualified route
leaks. An oversized combined worker report retains the scoped route sample
with unavailable results for the other vectors.

NetlinkRules adds IPv4 and IPv6 `RTM_GETRULE` dumps via libc and direct syscalls
on the same RAW/protected worker thread. Each dump checks a rule naming the VPN
fixture, a separate rule selecting the worker UID and an ordinary interface
control. The rules use `FR_ACT_NOP`, so matching them continues the kernel
lookup without selecting a routing table. The non-default table tag on the UID
rule exercises the filter without redirecting traffic. Core reserves unused
preferences and refuses existing GOTO references to them.
[Linux rule lookup](https://github.com/torvalds/linux/blob/v6.6/net/core/fib_rules.c#L275-L319)

Core syncs the rule intent before creation. Closing the TUN descriptors removes
the interfaces and their routes. Core deletes the exact rules separately and
confirms their absence in fresh dumps. On a same-boot restart the intent binds
cleanup to the recorded network namespace and selectors. A conflicting row
holds cleanup and subsequent runs. Route and rule verdicts are separate.
APK/Core RPC36 carries the paired task and observations. Export schema13
retains the rule subchecks alongside the route subchecks. Actual
Android coverage depends on the app domain's read permissions.

The Pixel 8 Pro with Android 17, Builtin and kernel 6.1.157 confirmed all
eight netlink route and rule vectors on the authentic worker. Explicit
`bind` on route netlink had produced `EACCES`; the unicast collector now
lets the kernel assign the local port at send time. Request authorization
and sender validation still apply. The same Android app domain was denied
reading `/proc/net/route` and `/proc/net/ipv6_route`, so those four
vectors remained unverified. [Android SELinux network policy](https://android.googlesource.com/platform/system/sepolicy/+/android16-release/private/net.te)

With the independent controls and reordered fixtures, the same Pixel also
confirmed interface flags, MTU, `getifaddrs`, the IPv4 address ioctl group
(including both `SIOCGIFCONF` size checks), and socket binding via libc and
direct syscalls. `RTM_GETLINK` and the proc groups remained unverified. A
separate access test on that Pixel used ordinary app processes targeting SDK
24, 27, 28, 29, 30 and 36. All six received `EACCES` from an unbound
`RTM_GETLINK` dump and from `/proc/net/{dev,if_inet6,route,ipv6_route}`.
Changing the diagnostic APK's target SDK would not recover those controls on
this ROM. Test them on a build that grants ordinary apps the relevant reads,
or in the existing isolated Linux oracles for kernel-only evidence. The
earlier [KMOD proc-interface run](archive/2026-11-03-proc-interface-fixture-qualification.md)
observed all four proc-interface leaks when the guest permitted those reads.
[Android route netlink policy](https://android.googlesource.com/platform/system/sepolicy/+/android16-release/private/net.te)

On Linux with user/network namespaces and `/dev/net/tun`, run the fixture test
from the repository root. It checks coexistence with the DOWN fixture, the
combined worker report, eight visible route vectors on an unfiltered kernel
and return to the original route table.

```sh
export VPNHIDE_HOST_NETNS="$(readlink /proc/self/ns/net)"
subuid_start="$(awk -F: -v user="$(id -un)" '$1 == user { print $2; exit }' /etc/subuid)"
unshare -Urn --map-users="10000:${subuid_start}:1" sh -ec 'ip link set lo up; cargo test --offline -p vpnhide_core_daemon --test route_fixture -- --ignored --test-threads=1'
```

The rule fixture test uses the mapped UID 10000 and checks ordinary teardown,
recovery after abrupt process exit and refusal to remove a conflicting rule.

The synchronous default-network check uses the real
`ConnectivityManager.getActiveNetwork()` Binder call. The source hook records
the original handle, then supplies the fixture VPN only to the authenticated
worker in RAW and Protected. Core requires a timed acknowledgement for each
phase, the VPN handle and transport in RAW, the real production replacement in
Protected and the same ordinary default network before and after injection.
A missing or changing ordinary default makes the check unverified. An observed
fixture VPN handle in both active phases remains a leak after cleanup failure.
This uses APK/Core RPC37 and the version3 framework channel. On a Pixel 8 Pro
with Android 17 and Builtin, RPC37 confirmed the default-network check in two
successive runs after reboot, using separate Android worker processes during
Direct Boot. Both runs reported 13 confirmed groups, seven unverified groups
and zero observed leaks. The framework authority and acknowledgement were
removed after the runs.
After unlocking the same boot, three of four runs reproduced that assessment.
One run reached the Android worker deadline during framework collection and
retained the eight confirmed native groups. The two immediate retries passed;
the cause of the intermittent timeout was not established.

The callback scenario opens independent Android LISTEN, default and REQUEST
registrations in each phase. On frameworks with the Bundle dispatcher, the
first real ordinary AVAILABLE for each authenticated worker registration
triggers synthetic VPN AVAILABLE and LOST ahead of the original event.
The two synthetic inputs go through the production router and the platform's
Messenger delivery. RAW must receive the VPN handle, transport and interface
marker and the original ordinary event. Protected LISTEN must suppress both
VPN events. Protected default and REQUEST must rebuild AVAILABLE from the
ordinary cover and keep it available when only the VPN is lost. The following
real AVAILABLE for the same cover must not produce a duplicate.
Core requires all three registrations to qualify the group. Android workers
unregister the callbacks after measurement. The paired RPC38 uses framework
channel version4 for the new observations and dispatch acknowledgements.

Pixel 8 Pro with Android 17 and Builtin confirmed the callback group in 29 of
32 repeated runs. The 16 runs following the two post-unlock worker deadlines
all confirmed 14 groups. A later run lost the framework acknowledgement in
RAW and retained eight native groups. No run reported a qualified leak. The
tested Core hot-switched after the hook reload and retained the same boot and
`system_server` generation. Both framework files and the temporary device
helper were removed.

The PendingIntent scenario opens independent LISTEN and REQUEST registrations
using package-scoped broadcasts. For each authenticated worker registration,
the first ordinary AVAILABLE enters the real PendingIntent dispatcher with a
detached VPN fixture before the original event. RAW must deliver the fixture
network. Protected LISTEN must suppress the fixture event and deliver the
original ordinary event. Protected REQUEST must deliver the ordinary cover in
`EXTRA_NETWORK`. The worker overlaps the two independent broadcast waits with
the PackageManager probes in the same phase. Android delivers the diagnostic
broadcasts on the worker's callback handler at foreground priority. The worker
verifies that the broadcast includes a request with
the expected INTERNET capability and unregisters both registrations and receivers.
Core requires one delivery per registration, ordinary controls before and after
the protected phase, and fixture cleanup. Both dispatch paths must qualify the
group. Paired RPC39 uses framework channel version5 for this evidence.

On Pixel 8 Pro with Android 17 and Builtin, PendingIntent and callbacks
qualified in 18 consecutive runs, each confirming 15 of 20 groups. The other
two runs in the 20-run series were a stale result and an Android worker
deadline. An earlier post-unlock run reached both protected dispatch branches
but did not receive the RAW broadcasts within the worker's observation window.
No run reported a qualified leak. The framework files and registrations were
gone after the series, and system_server's open descriptor count was unchanged.

When Apps hiding is configured, Core gives the worker a short-lived authority
for the installed Android Settings package in its own profile. The worker calls
`getPackageInfo`, `getApplicationInfo`, `getInstalledPackages` and
`getInstalledApplications` through Android PackageManager. The package must be
visible in the prepared, RAW and revoked phases. For the protected phase,
system_server adds only the fixture package to the worker's policy and applies
the same production filters. Core requires evidence that each filter saw the
real package and removed it, the worker still sees its own package, and both
lists preserve the same remaining package names. Each direct probe uses a
distinct phase-specific cache key and requires a system_server receipt to exclude
cache-only results. For its authenticated diagnostic worker, the hook filters
real PackageManager entries and sends the complete resulting name list in the
platform's original parcel wrapper. It omits metadata that the enumeration
probe does not measure. Direct lookups and other callers use the full platform
objects. The test leaves the installed packages and saved policy
unchanged. Paired RPC40 uses framework channel version6. Intent resolution, UID
mapping and other profiles are outside these four probes.

On Pixel 8 Pro with Android 17 and Builtin, all four package queries qualified
in 18 of 20 repeated runs. Those 18 runs confirmed 16 of 20 required groups.
One worker launch command timed out, and one run failed to read the revoked
framework acknowledgement. Three earlier post-unlock attempts expired on the
overall worker deadline. No run reported a qualified leak. After the series
both framework files were absent, with no descriptors open on either file.

In a later cold-boot Perfetto comparison on the same Pixel, a 2.43-second
installed-package query spent 2.02 seconds runnable and waiting for CPU.
With the name projection, the median of 24 calls per enumeration API fell
from 288 to 90 ms for packages and from 229 to 107 ms for applications.
Binder transactions fell from 10 to 2–3 and from 15 to 4, respectively. The
first automatic run after unlocking confirmed 16 groups in 13 seconds.
All 20 subsequent steady-state runs confirmed the same groups with no leaks
or infrastructure failures.

Ports still lacks controlled results when enabled. See the
[framework fixture requirements](rfcs/0002-autonomous-diagnostics.md#74-почему-callback--главный-технический-риск).
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
