# Detection vectors — what an app can probe, and who hides it

Authoritative coverage map for "how can an app tell a VPN is up (or that a
package is installed), and which component stops it?". If you add or change a
hook, update the matrix here so the picture stays in one place.

Scope: this file is about **detection vectors and their coverage**. For where
runtime state is *stored* see [state.md](state.md); for forward-looking work
see [ROADMAP.md](ROADMAP.md).

---

## 1. Threat model

A *target* app (banking, store, anti-fraud SDK, censorship-detector like
RKNHardering) runs on the user's device and tries to decide "is this device
behind a VPN?" or "is app X installed?". It is **unprivileged** (`untrusted_app`
SELinux domain, its own UID) and cannot load kernel modules or read other
apps' memory. Its toolbox is exactly the OS interfaces below.

vpnhide does **not** tear down the VPN. It makes those interfaces *lie* to
selected apps: the tunnel keeps working, but the listed targets see a device
that looks like it is on plain Wi-Fi/cellular, and (optionally) cannot see the
VPN-manager app at all.

Two recurring assumptions, both documented in ROADMAP:

- **Split tunnel** ([split-tunnel guidance](ROADMAP.md)): the recommended setup
  keeps target apps *outside* the tunnel, so handing them a physical network
  identity is consistent with where their traffic actually goes.
- Server-side signals (egress IP geolocation, DNS, latency) **cannot** be fixed
  on-device. vpnhide hides *local* VPN evidence only.

---

## 2. The enforcement layers

Coverage comes from vpnhide components plus the platform. They differ in
*where* they sit, *which APIs* they can touch, *how* targets are selected, and
*whether a raw-syscall detector can bypass them*.

| Layer | Where it runs | Selects targets by | Sees | Bypassable by raw syscalls? |
|---|---|---|---|---|
| **kmod** | Linux kernel (kretprobes + entry redirection) | **UID** (`/proc/vpnhide_ctl`) | Everything below libc — the syscall/socket/skb/seq_file source | **No** — filters at the source regardless of how userspace calls |
| **KPM** | Linux kernel (KernelPatch inline hooks) | **UID** (KPM `ctl0` supercall) | Same kernel sources as kmod, delivered through KernelPatch instead of `.ko` loading | **No** — filters at the source regardless of how userspace calls |
| **zygisk** | target process, inline `libc` hooks (shadowhook) | **package** (canonical JSON -> module-dir runtime wire, per-fork) | Only calls routed through the hooked `libc` symbols | **Yes** — a direct `svc #0` / unhooked libc entry slips past |
| **lsposed** | `system_server`, Binder hooks (Vector framework) | **package/appId** (`vpnhide_config.json` + `/data/system/packages.list`) | Java/framework Binder results *before* serialization to the app | N/A — the data is built in another process; the app only gets the sanitized parcel |
| **SELinux** | platform policy | domain (`untrusted_app`) | n/a — it *denies* access rather than filtering | n/a — a denial is a denial |

Key consequences:

- **The kernel backends (kmod `.ko` and KPM) are the bypass-proof native layers.**
  A detector reading `/proc/net/route` with a raw `openat` syscall, or driving
  netlink without libc, defeats Zygisk but not a kernel backend. kmod is the
  stable default for supported GKI kernels; KPM is the KernelPatch path for
  old/non-GKI kernels and `.ko` load failures.
- **lsposed is the only layer that can fake the high-level Java network model**
  (`ConnectivityManager`, `LinkProperties`, capabilities, callbacks) and
  **package visibility**. Native layers cannot synthesize a coherent
  `NetworkCapabilities` parcel; framework layers cannot touch `getifaddrs`.
- **kmod, KPM, and Zygisk are implementations of the same Native role — you
  install one active backend, not a stack.** The app prioritizes kmod > KPM >
  Zygisk when more than one is installed. `.ko` + KPM together is the dangerous
  case because they hook the same kernel functions and can freeze the device;
  other overlaps are redundant and make setup harder to reason about.
    - **kmod** is preferred on supported GKI kernels — bypass-proof and
      out-of-process, with the most test coverage; needs `CONFIG_KPROBES`.
    - **KPM** is also bypass-proof and out-of-process, but beta and dependent on
      KernelPatch runtime (APatch or KPatch-Next-Module).
    - **Zygisk** is the fallback where no kernel backend is practical — works on
      any arm64 kernel, but only catches libc-routed calls (a raw `svc #0` slips
      past) and runs in-process, visible to aggressive anti-tamper.
  The small deltas between backends — e.g. Zygisk also filters
  `/proc/net/{if_inet6,tcp,tcp6}`, kmod/KPM also handle the server host-route —
  are noted in the matrix; neither delta is a reason to stack native backends.
  (`RTM_GETRULE` used to be a kernel-only vector; Zygisk now parses it in the
  same recv netlink filter as `RTM_GETLINK`/`RTM_GETROUTE`.)
- So a complete install is **two components**: exactly one native backend (kmod,
  KPM, or Zygisk) **plus** lsposed — which covers both the Java network vectors
  and package visibility — with SELinux as an unreliable platform backstop
  underneath. Where only one layer covers a vector, that is called out below.

> **SELinux is a free but unreliable backstop.** On a Pixel 4a / Android 13,
> `untrusted_app` is already *denied* `read` on `/proc/net/route` and `search`
> on `/sys/class/net` (`avc: denied … permissive=0`). That closes some procfs
> vectors for free — but **SELinux policy is configured differently on every
> device and ROM**, so you cannot count on it:
>
> - It tightened over Android versions; older releases were far laxer about
>   `/proc/net/*` and `/sys/class/net`.
> - OEM and custom ROMs add their own `te` rules — some relabel or *grant*
>   accesses that stock AOSP denies, widening what an app can read.
> - A device booted `permissive` (many custom-kernel / dev setups), or an app
>   running in a more privileged domain, sees these paths wide open.
> - **The netlink path is not restricted** even on stock-enforcing devices, so
>   a detector just asks the kernel over `NETLINK_ROUTE` instead — exactly how
>   issue #86 surfaced.
>
> So a 🔒 below means "often denied on stock-enforcing builds", not "safe".
> Never treat a SELinux denial observed on one device as coverage; the vpnhide
> layers must stand on their own everywhere. (The same per-device caveat
> applies to the SELinux *labels* in [state.md](state.md).)

---

## 3. Coverage matrix

Legend: ✅ covered · ⚠️ partial / conditional · — not applicable to that layer ·
🔒 often closed by SELinux (varies by device).

### 3A. Interface enumeration — "is there a tun/wg/ppp interface?"

| Vector | How it manifests | kmod | KPM | Zygisk | lsposed | SELinux |
|---|---|:--:|:--:|:--:|:--:|:--:|
| `getifaddrs()` | native list of ifaces+addrs | ✅ via netlink hooks | ✅ via netlink hooks | ✅ unlinks VPN nodes | — | |
| `NetworkInterface.getNetworkInterfaces()` (Java) | JNI → `getifaddrs` | ✅ | ✅ | ✅ | — | |
| `ioctl(SIOCGIFNAME)` index→name | native | ✅ `dev_ioctl` | ✅ `dev_ioctl` | ✅ | — | |
| `ioctl(SIOCGIFCONF)` enumerate | native | ✅ `sock_ioctl` | ✅ `sock_ioctl` | ✅ `filter_ifconf` | — | |
| `ioctl(SIOCGIF{FLAGS,MTU,INDEX,HWADDR})` by name | native | ✅ `dev_ioctl` | ✅ `dev_ioctl` | ✅ pre-screen | — | `dev_ioctl` hooks the dispatcher, so the SIOCGIFHWADDR case (`dev_get_mac_address`, 5.4+) is covered too |
| `ioctl(SIOCGIF{ADDR,BRDADDR,DSTADDR,NETMASK})` by name | native | ✅ `devinet_ioctl` | ✅ `devinet_ioctl` | ✅ pre-screen | — | inet by-name ioctls take a separate path from `dev_ioctl`; hooked as of the finding-1 fix |
| netlink `RTM_GETLINK` dump | `recvmsg`/`recvfrom` of `RTM_NEWLINK` | ✅ `rtnl_fill_ifinfo` | ✅ `rtnl_fill_ifinfo` | ✅ filter by index | — | |
| netlink `RTM_GETADDR` dump | `RTM_NEWADDR` | ✅ `inet*_fill_ifaddr` | ✅ `inet*_fill_ifaddr` | ✅ filter by index | — | |
| `/sys/class/net/<iface>/*` | lookup/stat/open/readlink/readdir reveal iface nodes | ⚠️ optional reboot-gated VFS hooks | ⚠️ optional reboot-gated VFS hooks | ⚠️ optional best-effort libc hooks (incl. `readdir`/`getdents64` listing filter) | — | 🔒 usually denied |
| `/proc/sys/net/{ipv4,ipv6}/{conf,neigh}/<iface>` | lookup/stat/open/readlink/readdir reveal per-iface sysctl dirs | ⚠️ optional reboot-gated VFS hooks | ⚠️ optional reboot-gated VFS hooks | ⚠️ optional best-effort libc hooks | — | 🔒 usually denied |

Notes: bionic's `getifaddrs` itself runs over netlink, so the kernel-backend
`rtnl_fill_ifinfo` / `inet*_fill_ifaddr` hooks cover the Java and native paths
at once. Zygisk additionally unlinks entries from the `getifaddrs` result list
directly, so it works even if a future bionic stops using netlink.

Both kernel backends can additionally install four VFS entry redirects
(`filename_lookup`, `do_filp_open`, `vfs_getattr`, and `iterate_dir`) at boot.
They recognize resolved sysfs/procfs dentries, not pathname strings, so relative
`openat`, symlink-following, bind-mount, stat/access/readlink, open/fstat, and
directory-enumeration variants receive the same `ENOENT`/filtered view. This is
disabled by default because the redirect points are global hot paths even
though filtering remains per target UID. Enable **Hide VPN filesystem paths**
in Settings and reboot; when disabled the probes are not registered at all.

Zygisk projects the same optional feature into hook bit 27 and installs an
atomic process-local group for bionic `open*`, `stat`/`lstat`/`fstat`/`fstatat`,
`access*`, `readlink*`, and `readdir*` entry points. It recognizes canonical absolute paths
and relative `*at` paths resolved through the supplied directory fd, and takes
effect when the target app is restarted. This intentionally does **not** claim
the kernel contract: raw syscalls (including `getdents64`), unhooked libc
aliases, bind mounts, and paths reached through unrelated symlink aliases can
bypass it. Use `.ko` or KPM when those vectors matter.

**`SIOCGIFCONF` size-query subcase — closed in the `.ko`.** The classic two-step
`SIOCGIFCONF` (first call with `ifc_req == NULL` to learn the buffer size, then a
sized call to fill it) used to leak: the `.ko` filtered the fill but returned the
*unfiltered* length from the size query, so a target comparing the two back-to-back
saw a one-interface discrepancy. `sock_ioctl_ret` now also handles the
`ifc_req == NULL` path — it rcu-enumerates the socket's netns netdevs and subtracts
`sizeof(struct ifreq)` per *IPv4-addressed* VPN iface (matched by `ifa_label`, the
exact set/naming `inet_gifconf` would have emitted), so the size query agrees with
the filtered fill. Validated by the harness `ifconf_probe` vector.

**By-name `SIOCGIF*` range — the whole `0x8910`–`0x8970` family, not just the
common few.** The by-name row above covers every get-by-name ioctl, and the gate
must span the full range: `SIOCGIFINDEX` (0x8933, the ioctl `if_nametoindex()`
issues), `SIOCGIFTXQLEN` (0x8942), and `SIOCGIFMAP` (0x8970) all sit above the
"obvious" FLAGS/MTU/HWADDR cluster. A too-narrow ceiling leaks: `if_nametoindex`
returning a non-zero index for a hidden `vpn0` is a positive fingerprint. The
`.ko` avoids the trap entirely by filtering on the *returned* name with no cmd
gate; zygisk and KPM both use an explicit `0x8910..=0x8970` range check. (KPM
briefly capped at `0x8930` and leaked `SIOCGIFINDEX` before this was widened to
match.)

All three native implementations also clear the compacted part of the
kernel-written buffer before shortening `ifc_len`. Without that cleanup, a
caller could scan its oversized buffer past the reported length and recover a
removed VPN name (or detect duplicated entries left by compaction). The QEMU
probe starts with a zeroed oversized buffer and verifies that no interface
names remain in this tail; it does not require or assume that bytes beyond the
kernel's original result are overwritten.

**One narrow `SIOCGIFCONF` gap still open:**

- **32-bit (compat) callers (kmod `.ko`).** `sock_ioctl_krp` hooks `sock_ioctl`
  only; a 32-bit app's `SIOCGIFCONF` enters through `compat_sock_ioctl` →
  `compat_dev_ifconf` and never reaches it, so both the fill and the size-query
  filtering are skipped for 32-bit enumeration. Most current Android apps are
  64-bit and modern enumeration uses netlink (which the rtnl/inet fill hooks do
  cover), so this is narrow; closing it means also hooking `compat_sock_ioctl`
  with the compat `ifconf` layout. Deferred (low priority) — see
  [ROADMAP.md](ROADMAP.md).

The KPM backend's `filter_ifconf` compacts the fill the same way the `.ko` does,
but does not yet reduce the `ifc_req == NULL` size query (it would need raw netdev
walking by per-kver offset rather than the `.ko`'s rcu helpers); the size-query
subcase is therefore `.ko`-only for now and remains a known KPM gap.

### 3B. Route table — "is the default route a tunnel?"

| Vector | How it manifests | kmod | KPM | Zygisk | lsposed | SELinux |
|---|---|:--:|:--:|:--:|:--:|:--:|
| `/proc/net/route` (IPv4) | text, iface in col 1 | ✅ `fib_route_seq_show` | ✅ `fib_route_seq_show` | ✅ `filter_route_buf` | — | 🔒 often denied |
| `/proc/net/ipv6_route` | text, iface last field | ✅ `ipv6_route_seq_show` | ✅ `ipv6_route_seq_show` | ✅ | — | 🔒 |
| netlink `RTM_GETROUTE` **dump** | `RTA_OIF` index per route | ✅ `fib_dump_info` / `rt6_fill_node` | ✅ `fib_dump_info` / `rt6_fill_node` | ✅ `RTM_NEWROUTE` filter (issue #86) | — | |
| netlink `RTM_GETROUTE` **single** (`ip route get`) | one `rt_fill_info` reply | ⚠️ intentionally unhooked (see ROADMAP) | ⚠️ intentionally unhooked | ⚠️ not filtered | — | |
| netlink `RTM_GETRULE` (policy rules) | per-UID lookup tables | ✅ `fib_nl_fill_rule` | ✅ `fib_nl_fill_rule` | ✅ `rule_hides_vpn` (recv filter) | — | |
| host-route to the VPN **server** | `/32`·`/128` to a public IP via a *physical* iface | ✅ `is_public_host_route_via_physical` | ✅ `kpm_is_public_host_route{4,6}` | — | — | |
| `LinkProperties.getRoutes()` (Java) | framework route list | — | — | — | ✅ filter `mRoutes` | |

The **`if<N>` leak (issue #86)** lived here: a hidden tun still has an index, and
a route dump exposes that index even when the *name* is hidden, so the detector
renders it as the synthetic `if33`. Fixed in Zygisk by filtering `RTM_NEWROUTE`
on `RTA_OIF` *and* hooking the FORTIFY'd `recvfrom`/`__recvfrom_chk` that
fortified native code actually calls (plain `recv` is never emitted for a
fixed-size buffer). The kernel backends cover it via `fib_dump_info`.

**Host-route to the server** is a desktop-VPN pattern: OpenVPN / kernel
WireGuard / root VPNs pin a `/32` (or `/128`) route to the server's public IP
out the physical interface so tunnel packets don't loop. That route names a
*physical* iface, so name-based hiding misses it — the kernel backends match it by *shape*
(host prefix + public dst + physical dev) instead. **On Android `VpnService`
apps this route usually does not exist**: they reach the server via
`protect(socket)` (an `SO_MARK` fwmark that bypasses VPN routing), so no host
route is installed. This logic is therefore *inert* on the typical Android
setup and matters for desktop-style / non-`VpnService` VPNs. Kept because it is
cheap and correct when such a route is present; see issue discussion.

### 3C. Socket / address leaks — "any socket bound to a tunnel IP?"

| Vector | How it manifests | kmod | KPM | Zygisk | lsposed | SELinux |
|---|---|:--:|:--:|:--:|:--:|:--:|
| `setsockopt(SO_BINDTODEVICE)` / `SO_BINDTOIFINDEX` | bind success and `getsockopt` state reveal or select a hidden iface | ✅ pre-mutation entry redirect | ✅ pre-hook `skip_origin` | ⚠️ bionic `setsockopt` only | — | |
| `/proc/net/tcp` | local addr hex per socket | — | — | ✅ `filter_tcp4_buf` (by VPN addr) | — | 🔒 often denied |
| `/proc/net/tcp6` | 32-hex local addr | — | — | ✅ `filter_tcp6_buf` | — | 🔒 |
| `/proc/net/if_inet6` | IPv6 addrs, iface last field | — | — | ✅ `filter_if_inet6_buf` | — | 🔒 |

Since Linux 5.7, an unprivileged process can perform the first interface
bind, so `curl --interface tun0 ...` or a raw `setsockopt` syscall is a real
local oracle even without root. The kernel backends deny VPN names and indices
with `ENODEV` **before** `sk_bound_dev_if` changes. The `.ko` redirects
`sock_setsockopt` **and** `sk_setsockopt` (6.1+) to a typed wrapper because a
return-only kretprobe would be too late; KPM uses KernelPatch's pre-hook
`skip_origin`. Both symbols are hooked because a whole-kernel-LTO build can inline
`sock_setsockopt` into `__sys_setsockopt`, leaving `sk_setsockopt` the only symbol
on the syscall path — and the wrapper must not trust the ABI `level` argument
(LTO drops it as dead, since `sk_setsockopt` never reads it), because reaching
either function already proves the call is `SOL_SOCKET`. Both freeze the 5.9+
`sockptr_t` input before validation to close userspace TOCTOU. Below 5.9 the
option value is still a raw user pointer, so KPM hooks the resolved-ifindex
mutation helper instead — after the copy and the name lookup, before
`sk_bound_dev_if` changes. Which helper exists is a property of the tree, not of
the version: upstream renamed `sock_setbindtodevice_locked` to
`sock_bindtoindex_locked` in 5.8, and Android/vendor 5.4 trees backport the newer
name, so KPM probes both by symbol and takes whichever answers. A tree exporting
neither leaves the hook bit clear — the mask then says, honestly, that this
backend does not cover the vector.

Below 5.9 the KPM hook is a pre-hook on the mutation helper, so it runs before
that helper's own `CAP_NET_RAW` check. It therefore asks the same question first
(`capable(CAP_NET_RAW)`, no struct offsets involved) and denies only callers that
would otherwise have succeeded: where the kernel refuses everyone, every bind
keeps failing identically instead of singling the VPN name out with a different
errno. Same rule the Zygisk hook follows, arrived at from the same counterexample.

The kernel's own `CAP_NET_RAW` gate is **not** treated as a substitute. It used
to be: below 5.7 this vector was deliberately left unhooked on the grounds that
`sock_bindtoindex_locked()` rejects an unprivileged bind anyway. A LineageOS
5.4 build (sm8350) disproved it — the gate is compiled in there, and an
untrusted app still bound a socket to `tun0`. Whatever satisfies `ns_capable()`
on such a ROM, the lesson is that a kernel-side policy we do not control cannot
back a coverage claim.

When neither kernel backend is available, Zygisk inline-hooks bionic's
`setsockopt` entry point and applies the same pre-syscall `ENODEV` policy. It
copies the untrusted option value through a fault-contained self-read, so a bad
pointer still reaches the kernel for native `EFAULT` handling instead of
crashing the target process. This is deliberately **best effort**: a caller
issuing `__NR_setsockopt` through raw `svc #0` never enters bionic and bypasses
the hook. It no longer decides what to do from the release string. Instead it
measures, once per process, what this kernel returns for a bind to a name that
cannot exist, and denies a hidden interface with exactly that errno — EPERM on
trees that check `CAP_NET_RAW` before parsing the name, ENODEV on trees that
resolve the name first. Mirroring the measured answer is what keeps the reply
from being an oracle: where every bind is refused, a name-specific ENODEV would
announce the interface; where binds succeed, staying inert would leak it (the
LineageOS 5.4 case above). If the measurement is unusable the hook falls back to
the old version heuristic, so behaviour is never worse than before it existed.
`bind-probe` records the same value per kernel family in the QEMU lanes
(`bind_absent_name`).

This vector is deliberately tested by a raw `svc` probe. A second, non-target
UID inspects the same inherited socket after the target call, so a backend that
only rewrites errno while leaving the socket bound fails the test.

The three procfs rows remain **Zygisk-only** today: kernel backends hide IPv6
addresses on the netlink `RTM_GETADDR` path but do **not** hook `if_inet6_seq_show`,
`tcp4_seq_show`, or `tcp6_seq_show`, so the procfs equivalents leak under a
raw-syscall reader that SELinux happens to allow. Candidate kernel-backend work
if a real detector uses them.

### 3D. Framework network APIs (Java) — lsposed territory

All `system_server` Binder results, sanitized before they reach the app. Native
and kernel layers cannot fake these coherently. See
[lsposed/AGENTS.md](../lsposed/AGENTS.md) and `HookEntry.kt` for the full list;
summary:

| Vector | API | Sanitization (lsposed) |
|---|---|---|
| VPN transport flag | `NetworkCapabilities.hasTransport(TRANSPORT_VPN)` | strip `TRANSPORT_VPN`, add `NET_CAPABILITY_NOT_VPN`, clear `TransportInfo` |
| Active network is VPN | `getActiveNetwork()` / `Network.writeToParcel` | swap VPN `Network` handle for best physical one |
| Network enumeration | `getAllNetworks()` | drop VPN handles from the array |
| Legacy VPN type | `getNetworkInfo(TYPE_VPN)` / `getNetworkForType(TYPE_VPN)` | return `null`; otherwise disguise `NetworkInfo` `TYPE_VPN`→`TYPE_WIFI` |
| Interface name / routes / DNS | `LinkProperties.{getInterfaceName,getRoutes,getDnsServers}` | null `mIfaceName`, filter `mRoutes`, recurse into stacked links |
| Async push | `registerDefaultNetworkCallback()` / `registerNetworkCallback()` | suppress VPN-requested callbacks; stash recipient UID across dispatch (issue #70) |

### 3E. Package visibility — "is the VPN-manager app installed?"

lsposed-only, in `PackageVisibilityHooks.kt`, filtering `IPackageManagerBase`
results for observer appIds derived from canonical JSON against the hidden
package set in the same canonical JSON:

- enumeration: `getInstalledPackages`, `getInstalledApplications`
- resolution: `queryIntent{Activities,Services,Receivers,ContentProviders}`, `resolveIntent`, `resolveService`
- direct lookup: `getPackageInfo`, `getApplicationInfo`, `getInstaller{PackageName,SourceInfo}`
- UID mapping: `getPackageUid` (→ −1), `getPackagesForUid` (filtered)

System callers (UID < 10000) are always exempt so the launcher/installer keep
working. Self-lookups are exempt too: a hidden package is not hidden from callers
with the same appId, so a VPN app can protect itself with Apps hiding while still
being hidden from other observer apps.

---

## 4. Importance ranking

Rough priority when triaging "which vector matters", based on what real
detectors actually probe:

1. **Interface enumeration + route dump + interface bind (3A–3C).** The first thing every native
   detector checks. Must be airtight across libc *and* raw-syscall readers →
   needs a kernel backend (`.ko` or KPM) for the bypass-proof guarantee, Zygisk
   for kernel-backend-less installs.
2. **Framework network model (3D).** What pure-Java apps and most anti-fraud
   SDKs use. lsposed-only; high value because it is the easy path for apps.
3. **Package visibility (3E).** Stops "you have a VPN app installed" heuristics.
   Independent of whether a VPN is currently up.
4. **Socket/address procfs (3C).** Lower frequency; matters for thorough native
   detectors and only when SELinux allows the read.
5. **Single-lookup route, policy-rule edge cases (3B).** Rare; partially or
   intentionally uncovered (see Gaps).

---

## 5. Known gaps & assumptions

- **Raw-syscall native readers defeat Zygisk.** Only the kernel backends
  (`.ko`/KPM) are bypass-proof. Document any "covered" claim with the layer; a
  Zygisk-only install is not raw-syscall-proof.
- **Zygisk socket-interface binding coverage is libc-only.** Its `setsockopt`
  hook covers bionic-routed `SO_BINDTODEVICE` / `SO_BINDTOIFINDEX` calls, but a
  direct syscall bypasses any in-process libc hook. Use a kernel backend for a
  bypass-proof guarantee.
- **zygisk netlink filtering is recv-family-scoped.** It hooks `recvmsg`,
  `recv`, `recvfrom`, `__recvfrom_chk`. A detector reading a netlink socket via
  plain `read()`/`readv()` or `recvmmsg` would slip past. Not seen in the wild
  yet; add hooks if it appears.
- **Kernel-backend procfs gap:** no `if_inet6` / `tcp` / `tcp6` seq-file hooks
  in `.ko` or KPM (3C).
- **Filesystem hiding is optional in both kernel backends.** It must be selected
  before reboot so the hot VFS probes are absent entirely on default boots.
  Zygisk does not cover the sysfs/proc-sys interface-node vectors.
- **KPM SIOCGIFCONF size-query gap:** KPM compacts the returned ifreq array but
  does not yet reduce the `ifc_req == NULL` size query the `.ko` handles (3A).
- **Single-lookup route** (`rt_fill_info`) is intentionally unhooked — no stable
  static-function ABI; low value under split tunnel. See
  [ROADMAP.md](ROADMAP.md).
- **Host-route logic is inert under Android `VpnService`** (`protect()` path,
  3B). Kept for desktop-style VPNs.
- **Server-side signals are out of scope** (egress IP, DNS, RTT). Split-tunnel
  guidance is the mitigation, not code.
- **VPN-preserving mode** (keep target traffic *inside* the tunnel while hidden)
  is future work; today the physical-handle swap assumes split tunnel. See
  [ROADMAP.md](ROADMAP.md), issue #130.

---

## 6. Where coverage is defined in code

| Layer | Entry points |
|---|---|
| kmod | `kmod/vpnhide_kmod.c` (10 kretprobes + socket-bind redirects + four optional VFS redirects); iface matcher `kmod/generated/iface_lists.h` |
| KPM | `kmod/kpm/vpnhide_kpm.c` (KernelPatch inline hooks + ctl0, including four optional VFS hooks); offsets in `kmod/kpm/kver_offsets.h` |
| zygisk | `zygisk/src/hooks.rs` (ioctl/getifaddrs/openat/recv*); `zygisk/src/filter.rs` (procfs + netlink filters) |
| lsposed | `lsposed/app/.../hook/HookEntry.kt`, `hook/PackageVisibilityHooks.kt`; iface matcher `.../generated/IfaceLists.kt` |
| iface match rules | single source of truth `data/interfaces.toml` → `scripts/codegen-interfaces.py` renders all four targets (kmod/KPM C, zygisk Rust, lsposed native Rust, lsposed Kotlin) |

The interface-name patterns (`tun`/`tap`/`wg`/`ppp`/`ipsec`/`xfrm`/`utun`/`l2tp`/`gre`,
`tailscale`/`zt`/`he-ipv6`, substring `vpn`, and the `if<digits>` renamed-tunnel
form from issue #86) are generated identically for all layers from
`data/interfaces.toml`.
