# Diagnostics — how VPN Hide self-tests hiding and attributes the result

This is the **self-diagnosis** model: how the app checks that hiding actually
works for its own process and honestly reports *who* hid the VPN on each vector.
For the hiding side (which backend covers which detection vector) see
[detection-vectors.md](detection-vectors.md); for the app↔backend wire see
[protocol.md](protocol.md).

For the current execution states, screen/export dependencies, historical decisions
and unresolved semantic questions, see the
[diagnostics state analysis](diagnostics-state-analysis.md). That analysis does
not change the runtime contract. The proposed replacement is specified separately
in the [app state transition contract](app-state-transitions.md) and is not yet
implemented.

The [observation coordinator](observation-coordinator.md) now owns cache refreshes.
Dashboard cache derivation observes terminal diagnostics without implicitly
retrying Blocked/Failed; explicit refresh and existing diagnostic triggers still
use the current retry policy. This prevents diagnostic root refreshes from
recursively retriggering themselves through Dashboard invalidation. Measurement
classification and the completed-run retention policy below are unchanged.

Devices this was validated on: Pixel 4a (sunfish, Magisk, 4.14, kmod/KPM/Zygisk),
Pixel 8 Pro (husky, KernelSU-Next, GKI 6.1, KPM), and an Android 13 Zygisk device.

## 1. The problem it solves

A check result used to be a single tri-state boolean that conflated three unrelated
meanings of "pass": (a) the backend hid the VPN, (b) SELinux denied the probe
(EACCES), (c) there was nothing to leak on that surface. Counting all "passes" as
backend wins made an *installed-but-inactive* backend read as "Partial" (its
SELinux-blocked reads counted as passes) and a mostly-working Java layer read as
"Not working" (any one failing probe painted the whole layer red).

## 2. Root-differential — the reliable classifier

A clean probe result is ambiguous on its own: hook-suppressed and nothing-to-leak
look identical. Root (uid 0) is **not** a hook target, so a privileged read is the
ground truth for "what is actually on this surface". The app runs each native probe
twice — **in-process** (its own uid + SELinux domain + hooks) and **as root** — and
diffs them:

| root sees | app read | → outcome |
|---|---|---|
| nothing | — | **NothingToLeak** (empty ground truth wins, even over EACCES) |
| VPN | EACCES | **HiddenBySelinux** |
| VPN | ok, clean | **HiddenByBackend** |
| — | app saw VPN | **Leak** |

Priority: an empty ground truth is checked **before** EACCES — if root sees nothing,
the SELinux block is moot, it is simply nothing-to-leak.

**Ground truth is the same Rust probe binary run as root**, not shell `ip`/`cat`.
`GroundTruthProbe` extracts `vhprobe` from the APK, stages it to `/data/local/tmp`,
and execs it via `su`; it emits the same JSON as the in-process JNI path
(`run_all_json`), so the two views are directly comparable per check id. (This
replaced an earlier gobley/UniFFI binding — the whole native surface is now one
JSON-returning function built with plain cargo-ndk.)

The same Rust executable also has a read-only `--apatch-kpm-list` mode used by
the dashboard's installation-integrity check. Together with `kpatch kpm list`
on KPatch-Next, it detects a runtime-loaded `vpnhide` KPM when the
`/data/adb/modules/vpnhide_kpm` flashable module is absent. That combination
indicates that the user loaded or embedded the inner `vpnhide.kpm` file without
installing the complete `vpnhide-kpm.zip`; the app explains that the raw file
lacks the activator, boot scripts, and config delivery, then directs the user to
remove the raw KPM, install the ZIP through the root manager's Modules screen,
and reboot. APatch authentication uses the saved root-only SuperKey or its
trusted `su` token; the probe never prints either credential.

## 3. Per-check outcome (`CheckOutcome`)

`Leak` · `HiddenByBackend` · `HiddenBySelinux` · `NothingToLeak` ·
`NotMeasured(reason)`. Wire/log tokens: `leak`, `hidden_backend`, `hidden_selinux`,
`nothing_to_leak`, `not_measured_no_network`, `not_measured_no_ground_truth`.

The Rust probe reports `Pass` / `Fail` / `SelinuxBlocked` (EACCES/EPERM, no longer
folded into `Pass`) / `NetworkBlocked` (ECONNREFUSED from `socket()` — no network
permission). Native checks classify via the root differential above. **Java** checks
have no root differential (framework IPC), so they are binary — clean ⟹
`HiddenByBackend`, dirty ⟹ `Leak` — which is honest only because the self-in-tunnel
gate (§5) guarantees a VPN artifact was present to hide.

## 4. Layer status & verdict (dashboard tiles)

Each dashboard tile is a `LayerStatus`: `Absent` (no module installed) · `Inactive`
(installed, not loaded this boot) · `Active(hidden, leaks)`. Presence is decided
**before** the checks, so an unloaded backend can never render a verdict — it just
reads "not active" (this is the type-level fix for the old "Partial"). An `Active`
tile's verdict:

- `leaks == 0` → **Ok**
- `hidden > 0 && leaks > 0` → **Partial** (hides some, an owned vector still leaks)
- `hidden == 0 && leaks > 0` → **Broken** (loaded but suppressed nothing)

`hidden` must be a *measurement* (the root differential), never inferred from a clean
probe — otherwise Partial and Broken are indistinguishable. The native tile is judged
**only on vectors the active backend owns** (has a hook for): a leak on a not-owned
vector (e.g. `/proc/net/dev` under a kernel backend — no kernel hook exists) does not
turn the tile red. Such an **unowned leak is a surface no active backend can close on
this device**, so it also does **not** raise a dashboard warning or the "Issues" count
— alarming about a gap the user cannot act on is just noise (and support churn).
Instead it is shown neutrally ("not covered") in a separate group of the per-check
breakdown, so the residual surface stays honest without reading as a failure. The
only thing that raises the hero to *attention* is an **owned** leak — a vector the
active backend should hide but didn't (the user can act: report the device / switch
backend) — or a genuine module/version problem. So the **tile** answers "is this
module doing its job", and the dashboard stays clean whenever the active backend
hides everything it *can*. The Java tile uses the same rollup; LSPosed owns every Java
check, so all its leaks count.

## 5. Self-in-tunnel gate

Diagnostics are meaningless if VPN Hide itself is not routed through the VPN: split-
tunnelled out, there is no VPN artifact for its own probes to be hidden *from*, so
every check would read misleadingly clean. Before running any checks the app asks
the root snapshot which current Android networks are VPN networks. Interface-name
matching alone is not VPN presence: an `ipsec*` interface may belong to a carrier
IMS/IWLAN network explicitly marked `NOT_VPN`. Such interfaces remain hidden from
selected apps but do not activate the self-test gate. Only current NetworkAgentInfo
records count, not VPN requests, historical events or idle VPN-manager objects.
Unmanaged tunnels (for example root WireGuard) additionally require an up/unknown
interface with a non-local route. A failed network/route probe is a diagnostic
failure, not a claim that VPN is off or the app is excluded.

For the resulting candidate interfaces, `vhprobe --uid <selfUid> --vpn-ifaces
<comma-separated-ifaces>` (root, hook-inert) checks whether this uid is routed through
the VPN. Two passes over the policy rules (both address families): learn the VPN egress
table id(s) from rules that egress via a VPN interface (`oif tun*`), then check whether
a `uidrange` rule steers this uid into exactly that table. This is stricter than the
broad `netlink_getrule` diagnostic predicate — every uid sits in *some* per-network
table (wlan/rmnet, also non-standard), so it must pin the VPN table specifically. If
not routed, the UI shows an "add VPN Hide to your tunnel" prompt instead of clean
results. Table identifiers are scoped by address family; output-interface-only
rules are not UID membership. Native probe failures return `routed: null`, and the
gate reports a failed check instead of using a stale gate or assuming exclusion.

Unmanaged root tunnels may not have Android UID-range rules. If membership is not
found, read-only `ip route get ... uid <selfUid>` lookups sample their routed
prefixes in each family (public probe addresses for default routes). They send no
packets. A lookup using a candidate tunnel establishes routing; lookup errors are
inconclusive. This does not prove that every destination or socket mark uses the
tunnel, nor replace a full policy-routing evaluator.

## 6. Empirical facts that shape the checks

- **SELinux can carry part of "protection", and it is invisible without the
  differential.** `/proc/net/if_inet6` and `/proc/net/dev` still have no kernel
  backend hook. `/sys/class/net` and the per-interface `/proc/sys/net` trees are
  covered by a kernel backend only when the optional filesystem feature was
  enabled before reboot; Zygisk provides a weaker libc-routed version after the
  target process restarts. When SELinux denies a path first, the
  differential attributes that result to SELinux rather than overstating backend
  coverage. This is also why permissive devices need an explicit warning.
- **The VPN lives in protected sockets + per-UID policy tables, not the main route
  table.** A split-tunnel VPN app marks its sockets and installs `ip rule … uidrange
  <uid> lookup tun0`; it does *not* put a default route in the main table. So
  `/proc/net/route` (main table only) shows no VPN for the target and resolves to
  `NothingToLeak` — while **RTM_GETRULE** (policy rules) is the real routing detection
  vector, added as a probe mirroring the `fib_nl_fill_rule` kernel filter.
- **Some checks never fire** on any config: both `/proc/net/route` reads (native +
  Java) and the removed system-proxy check. They only added false confidence; the
  route reads are kept because the differential now labels them `NothingToLeak`
  honestly, the proxy check was dropped.
- **Suppression counters can distinguish "hook not loaded" from "hook not working"**
  (a per-hook Δ>0 during a probe is proof the hook did real work). They are **not**
  used by diagnostics — the root differential already gives the full 4-way + `hidden`
  without them — and stay only in the Statistics tab.

## 7. Native check → owning hook, verified on Pixel 4a

The native backend hooks map to the diagnostic checks below. Kernel backends
provide the strongest coverage; the optional Zygisk filesystem group covers
ordinary libc-routed probes but not raw syscalls or aliases. Full hiding matrix in
[detection-vectors.md](detection-vectors.md).

| check id | probes | kernel hook | notes |
|---|---|---|---|
| `ioctl_flags`, `ioctl_mtu` | `SIOCGIF*` by name | `dev_ioctl` | ENODEV for tun0 |
| `ioctl_conf` | `SIOCGIFCONF` | `sock_ioctl` | tun0 absent from ifconf |
| `getifaddrs`, `netlink_getlink` | RTM_GETLINK / getifaddrs | `rtnl_fill_ifinfo`, `inet*_fill_ifaddr` | |
| `so_bindtodevice` | `setsockopt(SO_BINDTODEVICE, tun0)` | `socket_bind_interface` | ENODEV = hidden; a bind that succeeds *and* `getsockopt` echoes `tun0` = leak |
| `netlink_getroute` | RTM_GETROUTE v4/v6 | `fib_dump_info`, `rt6_fill_node` | |
| `netlink_getrule` | RTM_GETRULE policy rules | `fib_nl_fill_rule` | v4+v6; kernel-only vector |
| `proc_route` | `/proc/net/route` | `fib_route_seq_show` | main table — empty for split-tunnel VPN |
| `proc_ipv6_route` | `/proc/net/ipv6_route` | `ipv6_route_seq_show` | |
| `proc_if_inet6` | `/proc/net/if_inet6` | **(none)** | no kernel seq_show hook — zygisk `openat` or SELinux only |
| `proc_dev` | `/proc/net/dev` | **(none)** | zygisk `openat` or SELinux only |
| `sys_class_net` | `/sys/class/net` | `filesystem_iface_paths` (`.ko`/KPM/Zygisk, optional) | kernel: resolved-dentry and reboot-gated; Zygisk: best-effort libc and restart-gated |
| `proc_sys_net` | `/proc/sys/net/*/{conf,neigh}` | `filesystem_iface_paths` (`.ko`/KPM/Zygisk, optional) | kernel: resolved-dentry and reboot-gated; Zygisk: best-effort libc and restart-gated |

The `so_bindtodevice` check probes this vector from the app process —
`setsockopt(SO_BINDTODEVICE, "tun0")`, classified by the same root differential.
To avoid blessing a broken return-only implementation on errno alone, a bind is a
leak **only** when it takes: `getsockopt` must echo `tun0` back (so a hook that
returns 0 without binding — or the kernel's own capability block — is not a false
positive), while `ENODEV` is the backend's pre-mutation denial. This catches the
common failure (a bindable VPN interface), but a single process cannot verify the
*pre-mutation* property itself — that the socket was never bound before `ENODEV`.
The QEMU `bind-probe` still performs that full state-level test with a raw syscall
and a second, non-target UID inspecting the inherited socket. Runtime deny hits
also appear in Statistics. Zygisk has its own libc-routed `zygisk_setsockopt` hook
but does not claim this vector in its owned-hook mask (so a leak here reads as an
unowned surface under Zygisk, not a tile failure) and emits no per-hook statistics
yet.

Java-level checks (LSPosed) cover the framework side — `hasTransport(VPN)`,
`NET_CAPABILITY_NOT_VPN`, `VpnTransportInfo`, `getAllNetworks`, `LinkProperties`,
`getNetworkForType(TYPE_VPN)`, the push `NetworkCallback` (issue #70), and the legacy
`getActiveNetworkInfo` / `getNetworkInfo(TYPE_VPN)` APIs.
