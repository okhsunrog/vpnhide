# Roadmap

This document tracks larger product directions that are too broad for a
single changelog entry. It is not a release commitment; concrete work should
still be tracked in GitHub issues and pull requests.

For the current state — which detection vectors exist and which component
covers each — see [detection-vectors.md](detection-vectors.md). This file is
the *forward-looking* counterpart: vectors that are partially covered or
intentionally deferred are described there and cross-linked from here.

## VPN Hiding Modes

### VPN-preserving concealment

Current Java `Network` handle hiding is optimized for the recommended
split-tunnel setup: target apps are kept outside the VPN, so returning a
physical `Network` handle is consistent with where their traffic should go.

There is a second valid use case: target apps must keep their traffic inside
the VPN while the VPN remains hidden from local detection APIs. This matters
for users who intentionally route banking, media, work, or home-network apps
through a VPN endpoint in a specific country.

The current physical-handle replacement can affect that use case when a target
app explicitly binds traffic to the returned `Network` with APIs such as
`bindProcessToNetwork`, `Network.bindSocket`, or `Network.openConnection`.
Ordinary sockets still follow Android VPN policy, but explicitly-bound traffic
may bypass the VPN on setups that allow physical-network use.

Follow-up work:

- Add a VPN-preserving concealment mode that hides VPN state without steering
  explicitly-bound target-app traffic away from the VPN.
- Decide whether this should be a global mode, per-target setting, or automatic
  behavior based on VPN bypassability / split-tunnel policy.
- Extend Diagnostics with a routing-oriented check that can distinguish
  "VPN hidden and traffic outside VPN" from "VPN hidden and traffic still
  inside VPN".

Tracking: [issue 130](https://github.com/okhsunrog/vpnhide/issues/130).

### Network handle edge cases

The physical replacement path used by LSPosed Java hooks intentionally prefers
connectivity over perfect concealment in rare fallback cases: if no non-VPN
replacement exists for `getActiveNetwork()`, the original active network is left
unchanged instead of reporting that there is no active network.

Follow-up work:

- Watch real app compatibility reports for APIs that are intentionally
  suppressed to `null`, especially `getNetworkForType(TYPE_VPN)`.
- Consider short-lived caching for the selected replacement network if real
  devices show measurable overhead from repeated `ConnectivityService` lookups.

## Kernel Module (kmod)

### Single-lookup route concealment (low priority)

The kmod hides VPN routes from netlink route *dumps* (`RTM_GETROUTE` with
`NLM_F_DUMP`, via the global `fib_dump_info` hook) and from `/proc/net/route`
and `/proc/net/ipv6_route`. It does **not** hide single resolved-route lookups
(`ip route get <dst>`, `RTM_GETROUTE` without `NLM_F_DUMP`), served by the
`static` `rt_fill_info`.

`rt_fill_info` is intentionally left unhooked. As a directly-called `static`
function it has no stable argument→register ABI: a fixed `regs[N]` read is
correct on some builds and wrong on others (verified to differ between a
real LTO device build and a no-LTO QEMU build, where `regs[3]` held
`table_id` instead of the `struct rtable *`). The vector is also low value —
detection apps enumerate routes via dumps (covered by `fib_dump_info`), and
single lookups respect the caller's own routing, which under the recommended
split-tunnel setup resolves to the physical interface anyway.

Follow-up work (low priority):

- If single-lookup concealment is ever wanted, hook the global `rtnl_unicast`
  (ABI-stable, runs in caller context) — the choke point for every
  single-reply rtnetlink response, IPv4 and IPv6 — and rewrite `RTA_OIF` in
  the reply skb to a physical ifindex, instead of reading a fixed register
  off the static `rt_fill_info`.

### 32-bit (compat) SIOCGIFCONF enumeration (low priority)

The kmod `.ko` filters `SIOCGIFCONF` interface enumeration via a kretprobe on
`sock_ioctl`, including the 64-bit `ifc_req == NULL` size-query path. A
**32-bit** app's `SIOCGIFCONF` still enters the kernel through
`compat_sock_ioctl` → `compat_dev_ifconf` and never reaches `sock_ioctl`, so its
enumeration is not filtered (it sees VPN interfaces). KPM also compacts the
filled ifreq array but does not yet reduce the `ifc_req == NULL` size query —
a separate, 64-bit gap, not the 32-bit/compat case this heading names.
Both remaining cases are tracked in [detection-vectors.md](detection-vectors.md)
(the SIOCGIFCONF notes).

This is **low priority**: most current Android apps are 64-bit, and modern
interface enumeration uses netlink `getifaddrs` (covered by the rtnl/inet fill
hooks), not the legacy `SIOCGIFCONF`. zygisk (its own `getifaddrs`/procfs
filtering) is unaffected.

Follow-up work (low priority — may revisit):

- Register a second kretprobe on `compat_sock_ioctl` and filter `SIOCGIFCONF`
  there using the compat `struct ifconf` / `struct ifreq` layout (32-bit
  pointers and `ifr_name` offset), mirroring the `sock_ioctl_ret` compaction.

## Diagnostics And Observability

- Add optional per-hook interception counters so users can see which apps are
  probing VPN state and through which API family.
- Keep install-time hook status detailed enough to diagnose Android framework
  drift, especially private-field changes in new Android releases.

### Permissive SELinux — dashboard severity and coverage

When the device is in SELinux Permissive mode, the dashboard raises a **warning**:
permissive exposes roughly six detection vectors VPN Hide relies on SELinux to
block (`RTM_GETROUTE`, `/proc/net/{tcp,tcp6,udp,udp6,dev,fib_trie}`,
`/sys/class/net`; see the coverage table in the top-level README). That warning
is the current, deliberate behaviour.

Open questions to settle later:

- **Severity.** Is a warning the right weight, or should it be an error (these
  vectors are genuinely uncovered while permissive) — or downgraded if the user
  has a backend that already covers them another way?
- **Behaviour.** Should VPN Hide do anything beyond warning under permissive —
  e.g. attempt to cover the SELinux-relied vectors at the app/native layer
  instead of leaving them to SELinux, or surface which specific vectors are
  currently open?
- **Detection.** `getenforce` is global; per-domain permissive (`permissive`
  type rules) isn't caught. Worth deciding whether that matters.

The Statistics screen surfaces the per-uid × per-hook interception counters
every active backend already reports (Java + the one installed native backend),
aggregated per app and labelled by detection method. A capture-session mode
baselines the counters and shows live deltas while the user exercises a target
app — "start capture → open the app → see which methods it probed", without any
per-event timestamps or persistence (counters are u64; a backend restart that
drops counts below the baseline is detected and re-baselined).

This is intentionally scoped to **target apps only** — the apps the user already
protects, which are the only ones the hooks currently count. That is where the
value is: confirming the hiding works and seeing which protected apps probe
hardest.

Follow-up work (low priority):

- Monitoring / capture of **non-target** apps — counting probes from callers the
  user has *not* added, to discover new apps attempting VPN detection
  ([issue 119](https://github.com/okhsunrog/vpnhide/issues/119)). This needs an
  opt-in "observe" mode in the Java backend (system_server sees every caller;
  kernel backends only count their target UIDs, and Zygisk is only injected into
  its own targets), plus write-coalescing on the hot path. Deferred until the
  target-app statistics prove their value.
### Zygisk per-app statistics — intentionally not supported

The Statistics screen has no native per-app counters when Zygisk is the active
backend (it shows an explanatory banner; the Java/LSPosed counters are still
reported). This is a deliberate non-feature, not a gap to fill.

Zygisk hooks run inside each target app's own process under that app's SELinux
sandbox (`untrusted_app`, the app's UID). Such a process cannot write anywhere
VPN Hide can read it back — not VPN Hide's private dir, not `/data/adb`. The
existing zygisk heartbeat only works because VPN Hide targets *itself*, so its
own injected process writes to its own data dir. Exporting per-app counters from
*other* apps' injected processes would require one of:

- writing a stats file into the **target app's own sandbox** — which that app
  can list and use as a "I'm being hooked" detection signal, directly
  undercutting the stealth this whole project exists for; or
- a shared world-writable location plus a `sepolicy.rule` granting
  `untrusted_app` write — extra SELinux surface and moving parts.

Both degrade stealth or add attack surface for the **least-preferred, most-
detectable** native backend (priority is kmod > KPM > Zygisk). KPM covers the
same native vectors invisibly and already reports in-kernel counters, so the
right path for a Zygisk user who wants native statistics is to move to KPM.
Revisit only if Zygisk itself is kept long-term; the likelier outcome is that
Zygisk is retired once KPM is proven and its users have migrated.

## Configuration

- Consider user-defined VPN interface prefixes for uncommon tunnels or renamed
  interfaces, while keeping generated defaults as the primary path.
- Keep split-tunnel guidance prominent in user-facing docs because server-side
  IP, DNS, and latency checks cannot be fixed client-side.
