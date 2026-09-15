# vpnhide -- Kernel module

Kernel-probe module that hides VPN interfaces from selected apps. Part of [vpnhide](../README.md).

> **Just using the app?** The Dashboard names the exact zip for your kernel and walks you through installing it — see **[Install the kernel module](../docs/help/en/kmod-install.md)** (or the in-app **Help & guide**). This README is for building and understanding the module.

The module does not modify the target app's process: there are no userspace
function patches, injected framework classes, or module-owned anonymous memory
regions. Detection paths outside the hooks listed below remain out of scope.

## What it hooks

| Hook target | What it filters | Detection path covered |
|---|---|---|
| `dev_ioctl` | `SIOCGIFFLAGS`, `SIOCGIFNAME`, and other per-interface ioctls: returns `-ENODEV` for VPN interfaces | Direct `ioctl()` calls from native code (Flutter/Dart, JNI, C/C++) |
| `sock_ioctl` | `SIOCGIFCONF`: compacts VPN entries out of the returned interface array | Interface enumeration via `ioctl(SIOCGIFCONF)` |
| `rtnl_fill_ifinfo` | Trims VPN entries from RTM_NEWLINK netlink dumps via `skb_trim` and returns 0 | `getifaddrs()` (which uses netlink internally), any netlink-based interface enumeration |
| `inet6_fill_ifaddr` | Trims VPN entries from RTM_GETADDR IPv6 responses via `skb_trim` | IPv6 address enumeration over netlink |
| `inet_fill_ifaddr` | Trims VPN entries from RTM_GETADDR IPv4 responses via `skb_trim` | IPv4 address enumeration over netlink |
| `fib_route_seq_show` | Forward-scans for VPN lines and compacts them out with `memmove` | `/proc/net/route` reads |
| `ipv6_route_seq_show` | Forward-scans for VPN lines and compacts them out with `memmove` | `/proc/net/ipv6_route` reads |
| `fib_dump_info` | Trims IPv4 VPN route entries and public physical-interface host-route hints from netlink route dumps via `skb_trim` | RTM_GETROUTE route table dumps |
| `rt6_fill_node` | Trims IPv6 VPN route entries from netlink route dump replies via `skb_trim` | IPv6 RTM_GETROUTE dumps |
| `fib_nl_fill_rule` | Trims target-UID policy rules and VPN interface rules from netlink rule dumps via `skb_trim` | RTM_GETRULE policy routing dumps |
| `sock_setsockopt` / `sk_setsockopt` entry redirect | Returns `-ENODEV` for hidden VPN names and indices before socket state changes | Raw `SO_BINDTODEVICE` / `SO_BINDTOIFINDEX` calls |
| `filename_lookup`, `do_filp_open`, `vfs_getattr`, `iterate_dir` entry redirects (optional) | Makes VPN interface nodes look absent under sysfs and `/proc/sys/net/*/{conf,neigh}` | `stat`/`access`/`readlink`, `open`/`fstat`, and `getdents64` probes |

All filtering is **per-UID**: only processes whose UID is a `target` in the config written to `/proc/vpnhide_ctl` see the filtered view. Everyone else (system services, VPN client, NFC subsystem) sees the real data.

The VFS row is additionally **reboot-gated and disabled by default**. Enable
**Settings → Experimental protection → Hide VPN filesystem paths** and reboot.
When disabled, those four probes are not registered, so the default boot pays
no VFS hot-path trampoline cost. The same canonical setting also controls the
equivalent optional KPM hooks.

### Optional filesystem hook loader contract

`filesystem_hiding` is a read-only boolean module parameter consumed only by
`insmod`; it is not a control-v2 field. The shipped activator's `boot-load`
command reads canonical optional feature `filesystem_iface_paths` and passes
`filesystem_hiding=1` or `filesystem_hiding=0` accordingly. Omitting the
parameter has the same disabled behavior as `0`.

When enabled, module init registers `filename_lookup`, `do_filp_open`,
`vfs_getattr`, and `iterate_dir` as one optional group. A registration failure
rolls the whole group back; telemetry then omits hook bit 27 and reports a
partial install. The control-v2 target mask remains a separate per-UID gate:
loading the group does not hide paths for targets whose mask omits
`filesystem_iface_paths`. Because the parameter is init-only and the module is
not unloadable, changing it requires a reboot.

## Why kernel-level?

Some anti-tamper SDKs read `/proc/self/maps` via raw `svc #0` syscalls (bypassing any libc hook) and check ELF relocation integrity. No userspace interposition can hide from them.

Kernel kretprobes modify kernel function behavior, not userspace code. The target app's process memory, ELF tables, and `/proc/self/maps` are completely untouched.

## GKI compatibility

The exported symbols used by the module are part of the GKI KMI. A build targets
one GKI generation and its matching headers/CRCs; the C source stays identical
across generations.

CI builds are provided for all 7 GKI generations: `android12-5.10` through `android16-6.12`.

For old/non-GKI kernels or devices where the `.ko` cannot load, see the
[KPM backend](kpm/README.md). It covers the same kernel-level Native role through
KernelPatch inline hooks and is packaged as `vpnhide-kpm.zip`.

## Build

See [BUILDING.md](BUILDING.md) for the full guide (DDK Docker build, kernel source preparation, toolchain setup, `Module.symvers` generation).

```bash
./kmod/build.py --kmi android14-6.1
```

## Install

1. `adb push vpnhide-kmod-<kmi>.zip /sdcard/Download/` (download the zip matching your device's GKI generation, e.g. `vpnhide-kmod-android14-6.1.zip`)
2. KernelSU-Next manager -> Modules -> Install from storage
3. Reboot

The loaded `.ko` intentionally remains resident until reboot. Disable, update,
or remove it through the root module manager and reboot to apply that change;
ordinary `rmmod` is not supported.

On boot:

- `post-fs-data.sh` execs the Rust activator's bounded `boot-load` phase, which
  reads the reboot-gated setting, runs `insmod`, and writes load diagnostics.
- `service.sh` starts the activator's late-start `boot-service` phase in the
  background and immediately returns to the root manager. The activator reads
  `/data/system/vpnhide_config.json`, enumerates Android users, resolves package
  names for each user separately, and emits a `vpnhide 2 config` snapshot
  ([protocol](../docs/protocol.md)) to `/proc/vpnhide_ctl`.

### Target management

**VPN Hide app (recommended):** open the VPN Hide app (the [lsposed](../lsposed/) APK). It lists all installed apps with icons, search, and checkboxes. Saves the canonical JSON config and runs the installed native activator immediately. Works on KernelSU, Magisk, and APatch. Managing targets this way is all a normal setup needs; the shell equivalents are in the collapsible **For developers / advanced usage** section at the end of this file.

The app first writes `/data/system/vpnhide_config.json`, the persistent
package-keyed desired state, and then invokes the selected native activator. The
kmod activator derives `/proc/vpnhide_ctl` from that JSON; LSPosed independently
reads the canonical JSON from `system_server`.

## Combined use with system_server hooks

Covering both native and Java API detection paths requires two layers, without
placing vpnhide hooks in the target app's process:

- **vpnhide-kmod** (this module) covers the native side: `ioctl`, `getifaddrs()` (netlink), `/proc/net/route`, `/proc/net/ipv6_route`, netlink address/route/rule dumps, pre-mutation `SO_BINDTODEVICE` / `SO_BINDTOIFINDEX` denial, and optional sysfs/proc-sys interface-path concealment.
- **[lsposed](../lsposed/)** hooks `writeToParcel()` on `NetworkCapabilities`, `NetworkInfo`, `LinkProperties` inside `system_server` -- stripping VPN data before Binder serialization reaches the app.

Together they cover the detection paths documented in
[`docs/detection-vectors.md`](../docs/detection-vectors.md). That document also
lists known gaps and environment-dependent signals.

KPM is the other kernel-level Native backend for this same role. Do not run KPM
and the `.ko` at the same time; choose one kernel backend, then pair it with
LSPosed for Java APIs.

### Setup

1. Install **vpnhide-kmod** as a KSU module (this module).
2. Install **[lsposed](../lsposed/)** as an LSPosed/Vector module and add **"System Framework"** to its scope (no other apps in scope).
3. Pick target apps in the VPN Hide app -- it manages targets for both the kernel module and the system_server hooks.

## Architecture notes

### Why kretprobes work here

kretprobes instrument kernel functions by replacing their return address on the stack. Unlike userspace inline hooks (which modify instruction bytes), kretprobes:

- Don't modify the target function's code in a way visible to userspace -- `/proc/self/maps` and the function's ELF bytes are unchanged
- Keep the instrumentation outside the target app's process; kernel-level
  observability still depends on the device's access controls and debug surface
- Can target eligible non-inlined functions with available symbols, including
  static functions; kprobe blacklists and compiler inlining can still prevent
  registration or leave a symbol off the live path

### dev_ioctl calling convention (GKI 6.1, arm64)

```c
int dev_ioctl(struct net *net,       // x0
              unsigned int cmd,       // x1
              struct ifreq *ifr,      // x2 -- KERNEL pointer
              void __user *data,      // x3 -- userspace pointer
              bool *need_copyout)     // x4
```

**Important:** `x2` is a kernel-space pointer (the caller already did `copy_from_user`). Using `copy_from_user` on it will EFAULT on ARM64 with PAN enabled. The return handler reads via direct pointer dereference.

### Why sock_ioctl, not dev_ifconf, for SIOCGIFCONF

`SIOCGIFCONF` does NOT go through `dev_ioctl()`. The call path is `sock_ioctl → dev_ifconf()` -- a completely separate function from `dev_ioctl`, which handles `SIOCGIFFLAGS`, `SIOCGIFNAME`, etc.

The natural choice would be to hook `dev_ifconf` directly, but Clang LTO can inline it into `sock_do_ioctl` while leaving an unused `dev_ifconf` symbol in `kallsyms`. A kretprobe can then register successfully without observing the live path. On 6.1+, `SIOCGIFCONF` is dispatched directly from `sock_ioctl`, so `sock_do_ioctl` is not a cross-version hook point either.

`sock_ioctl` is the stable hook point because (1) it is the `file_operations->unlocked_ioctl` callback for socket fds and therefore remains address-taken; (2) the supported GKI paths dispatch socket ioctls through it; and (3) after it returns, the ifconf data (ifreq array + `ifc_len`) is already in userspace, so the module can filter it uniformly via `copy_from_user`/`copy_to_user`.

The entry handler stashes the userspace `argp`; the return handler reads back the buffer, compacts out VPN entries, and updates `ifc_len` via `put_user`. Cost is one `cmd == SIOCGIFCONF` compare per socket ioctl for non-target paths.

### rtnl_fill_ifinfo / inet_fill_ifaddr / inet6_fill_ifaddr: skb_trim

All three netlink fill functions are skipped the same way: the entry handler saves `skb->len` before the fill writes anything; the return handler calls `skb_trim(skb, saved_len)` to undo whatever was written, then returns 0 (success). The dump iterator sees a successful entry of zero new bytes and advances to the next interface/address.

We do **not** return `-EMSGSIZE` to skip a VPN entry. On Android 14 / 6.1 GKI kernels, the dump iterator interprets `-EMSGSIZE` on an empty skb as "buffer too small for even one entry" and retries the same entry forever — observed in production as a hang of `getifaddrs()` (issue #38). The `skb_trim`-and-return-0 path avoids the retry loop on every netlink dump function uniformly.

### fib_route_seq_show: seq_file buffer compaction

`fib_route_seq_show(struct seq_file *seq, void *v)` appends one or more tab-separated route lines to `seq->buf`. Each call can write multiple lines (one per `fib_alias` in the routing table entry).

The kretprobe entry handler saves the `seq` pointer and `seq->count` (current buffer position) in `ri->data`. The return handler scans the newly written region `[saved_count, seq->count)` line by line, extracts the first tab-delimited field (interface name), and compacts out VPN lines using `memmove`. Finally, `seq->count` is adjusted to reflect the reduced content.

**Why we save seq in the entry handler:** in a kretprobe return handler, `regs->regs[0]` (x0 on arm64) contains the function's *return value*, not the original first argument. The original code tried to read `seq` from x0 in the return handler, which was reading the return value (0) as a pointer -- a bug that would crash or silently fail. The fix is standard kretprobe practice: save arguments in `ri->data` during the entry handler.

<details>
<summary><strong>For developers / advanced usage</strong></summary>

You do not need these to use the module — the VPN Hide app writes the canonical
config and runs the activator for you. They are shell equivalents for
development and debugging.

```bash
# Edit /data/system/vpnhide_config.json, then run the module activator.
adb shell su -c '/data/adb/modules/vpnhide_kmod/activator'

# Or push a control-config snapshot straight to the kernel (docs/protocol.md):
# header + folded debug flag + grouped target UIDs + mandatory end count
# (a0003ff = shared kernel hooks plus the optional .ko filesystem hook;
# control v2 uses bare hex).
adb shell su -c 'printf "vpnhide 2 config\ndebug 0\ntargets a0003ff 28b7\nend 1\n" > /proc/vpnhide_ctl'
```

</details>

## License

MIT. See [LICENSE](../LICENSE).

The compiled module declares `MODULE_LICENSE("GPL")` as required by the Linux kernel to resolve `EXPORT_SYMBOL_GPL` symbols (`register_kretprobe`, `proc_create`, etc.) at runtime.
