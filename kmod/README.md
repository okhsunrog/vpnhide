# vpnhide -- Kernel module

kretprobe-based kernel module that hides VPN interfaces from selected apps. Part of [vpnhide](../README.md).

Zero footprint in the target app's process -- no modified function prologues, no framework classes, no anonymous memory regions. Invisible to aggressive anti-tamper SDKs.

## What it hooks

| kretprobe target | What it filters | Detection path covered |
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

All filtering is **per-UID**: only processes whose UID is a `target` in the config written to `/proc/vpnhide_ctl` see the filtered view. Everyone else (system services, VPN client, NFC subsystem) sees the real data.

## Why kernel-level?

Some anti-tamper SDKs read `/proc/self/maps` via raw `svc #0` syscalls (bypassing any libc hook) and check ELF relocation integrity. No userspace interposition can hide from them.

Kernel kretprobes modify kernel function behavior, not userspace code. The target app's process memory, ELF tables, and `/proc/self/maps` are completely untouched.

## GKI compatibility

All symbols used (`register_kretprobe`, `proc_create`, `seq_read`, etc.) are part of the stable GKI KMI, so the same `Module.symvers` CRCs work across all devices running the same GKI generation. The C source is identical across generations -- only the kernel headers and CRCs differ.

CI builds are provided for all 7 GKI generations: `android12-5.10` through `android16-6.12`.

For old/non-GKI kernels or devices where the `.ko` cannot load, see the
[KPM backend](kpm/README.md). It covers the same kernel-level Native role through
KernelPatch inline hooks and is packaged as `vpnhide-kpm.zip` (beta).

## Build

See [BUILDING.md](BUILDING.md) for the full guide (DDK Docker build, kernel source preparation, toolchain setup, `Module.symvers` generation).

```bash
./kmod/build.py --kmi android14-6.1
```

## Install

1. `adb push vpnhide-kmod.zip /sdcard/Download/`
2. KernelSU-Next manager -> Modules -> Install from storage
3. Reboot

On boot:
- `post-fs-data.sh` runs `insmod` to load the kernel module
- `service.sh` runs the Rust activator, which reads `/data/system/vpnhide_config.json`, resolves package names via `pm list packages -U --user all`, and emits a `vpnhide 1 config` snapshot (docs/protocol.md) to `/proc/vpnhide_ctl`

### Target management

**VPN Hide app (recommended):** open the VPN Hide app (the [lsposed](../lsposed/) APK). It lists all installed apps with icons, search, and checkboxes. Saves the canonical JSON config and runs the installed native activator immediately. Works on KernelSU, Magisk, and APatch.

**Shell:**
```bash
# Edit /data/system/vpnhide_config.json, then run the module activator.
adb shell su -c '/data/adb/modules/vpnhide_kmod/activator'

# Or push a control-config snapshot straight to the kernel (docs/protocol.md):
# header + folded debug flag + one target line per UID (0x3ff = all hooks).
adb shell su -c 'printf "vpnhide 1 config\ndebug 0\ntarget 0x28b7 0x3ff\n" > /proc/vpnhide_ctl'
```

The app writes to **two layers** simultaneously:
1. `/data/system/vpnhide_config.json` -- persistent package-keyed roles (survives module updates and reboots)
2. runtime channels derived from it -- `/proc/vpnhide_ctl` for the kernel module, direct canonical self-read for the [lsposed](../lsposed/) module's system_server hooks

## Combined use with system_server hooks

For apps with aggressive anti-tamper SDKs, full VPN hiding requires covering both native and Java API detection paths -- without placing any hooks in the target app's process:

- **vpnhide-kmod** (this module) covers the native side: `ioctl`, `getifaddrs()` (netlink), `/proc/net/route`, `/proc/net/ipv6_route`, netlink address enumeration, netlink route dumps, and policy routing rule dumps.
- **[lsposed](../lsposed/)** hooks `writeToParcel()` on `NetworkCapabilities`, `NetworkInfo`, `LinkProperties` inside `system_server` -- stripping VPN data before Binder serialization reaches the app.

Together they provide complete VPN hiding without any hooks in the target app's process.

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
- Can't be detected by the target app -- the app can only inspect its own process memory, not kernel data structures
- Work on any function visible in `/proc/kallsyms`, including static (non-exported) functions

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

The natural choice would be to hook `dev_ifconf` directly, but on GKI 5.10 (Clang LTO) the linker inlines `dev_ifconf` into `sock_do_ioctl`. The `dev_ifconf` symbol stays in `kallsyms` as a dead stub, so `register_kretprobe` succeeds but the probe never fires. Confirmed by disassembly on Xiaomi 13 Lite (5.10.136) and Lenovo Legion 2 Pro (5.10.101): no `bl dev_ifconf` in `sock_do_ioctl`. On 6.1+, `SIOCGIFCONF` was moved out of `sock_do_ioctl` and is dispatched directly from `sock_ioctl`, so hooking `sock_do_ioctl` would miss it on newer kernels too.

`sock_ioctl` is the correct hook point because (1) it is the `file_operations->unlocked_ioctl` callback for socket fds — used as a function pointer, so LTO can never inline it; (2) all socket ioctls, including `SIOCGIFCONF`, pass through it on every kernel version (5.10 through 6.12+); (3) after `sock_ioctl` returns, the ifconf data (ifreq array + `ifc_len`) is already in userspace, so we filter it uniformly via `copy_from_user`/`copy_to_user` regardless of kernel version.

The entry handler stashes the userspace `argp`; the return handler reads back the buffer, compacts out VPN entries, and updates `ifc_len` via `put_user`. Cost is one `cmd == SIOCGIFCONF` compare per socket ioctl for non-target paths.

### rtnl_fill_ifinfo / inet_fill_ifaddr / inet6_fill_ifaddr: skb_trim

All three netlink fill functions are skipped the same way: the entry handler saves `skb->len` before the fill writes anything; the return handler calls `skb_trim(skb, saved_len)` to undo whatever was written, then returns 0 (success). The dump iterator sees a successful entry of zero new bytes and advances to the next interface/address.

We do **not** return `-EMSGSIZE` to skip a VPN entry. On Android 14 / 6.1 GKI kernels, the dump iterator interprets `-EMSGSIZE` on an empty skb as "buffer too small for even one entry" and retries the same entry forever — observed in production as a hang of `getifaddrs()` (issue #38). The `skb_trim`-and-return-0 path avoids the retry loop on every netlink dump function uniformly.

### fib_route_seq_show: seq_file buffer compaction

`fib_route_seq_show(struct seq_file *seq, void *v)` appends one or more tab-separated route lines to `seq->buf`. Each call can write multiple lines (one per `fib_alias` in the routing table entry).

The kretprobe entry handler saves the `seq` pointer and `seq->count` (current buffer position) in `ri->data`. The return handler scans the newly written region `[saved_count, seq->count)` line by line, extracts the first tab-delimited field (interface name), and compacts out VPN lines using `memmove`. Finally, `seq->count` is adjusted to reflect the reduced content.

**Why we save seq in the entry handler:** in a kretprobe return handler, `regs->regs[0]` (x0 on arm64) contains the function's *return value*, not the original first argument. The original code tried to read `seq` from x0 in the return handler, which was reading the return value (0) as a pointer -- a bug that would crash or silently fail. The fix is standard kretprobe practice: save arguments in `ri->data` during the entry handler.

## License

MIT. See [LICENSE](../LICENSE).

The compiled module declares `MODULE_LICENSE("GPL")` as required by the Linux kernel to resolve `EXPORT_SYMBOL_GPL` symbols (`register_kretprobe`, `proc_create`, etc.) at runtime.
