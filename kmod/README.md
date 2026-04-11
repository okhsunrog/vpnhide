# vpnhide -- Kernel module

kretprobe-based kernel module that hides VPN interfaces from selected apps. Part of [vpnhide](../README.md).

Zero footprint in the target app's process -- no modified function prologues, no framework classes, no anonymous memory regions. Invisible to aggressive anti-tamper SDKs.

## What it hooks

| kretprobe target | What it filters | Detection path covered |
|---|---|---|
| `dev_ioctl` | `SIOCGIFFLAGS`, `SIOCGIFNAME`: returns `-ENODEV` for VPN interfaces. `SIOCGIFCONF`: compacts VPN entries out of the returned array. | Direct `ioctl()` calls from native code (Flutter/Dart, JNI, C/C++) |
| `rtnl_fill_ifinfo` | Returns `-EMSGSIZE` for VPN devices during RTM_GETLINK netlink dumps, causing the kernel to skip them | `getifaddrs()` (which uses netlink internally), any netlink-based interface enumeration |
| `fib_route_seq_show` | Rewinds `seq->count` to hide lines with VPN interface names | `/proc/net/route` reads |

All filtering is **per-UID**: only processes whose UID appears in `/proc/vpnhide_targets` see the filtered view. Everyone else (system services, VPN client, NFC subsystem) sees the real data.

## Why kernel-level?

Some anti-tamper SDKs read `/proc/self/maps` via raw `svc #0` syscalls (bypassing any libc hook) and check ELF relocation integrity. No userspace interposition can hide from them.

Kernel kretprobes modify kernel function behavior, not userspace code. The target app's process memory, ELF tables, and `/proc/self/maps` are completely untouched.

## GKI compatibility

The module is built against the Android Common Kernel (ACK) source for `android14-6.1`. All symbols it uses (`register_kretprobe`, `proc_create`, `seq_read`, etc.) are part of the stable GKI KMI, so the same `Module.symvers` CRCs work across all devices running the same GKI generation.

KernelSU bypasses the kernel's vermagic check, so no runtime patching is needed. `post-fs-data.sh` simply runs `insmod` directly.

### Current build target

- **`android14-6.1`** -- Pixel 8/9 series, Samsung Galaxy S24/S25, OnePlus 12/13, Xiaomi 14/15, and most 2024 flagships on Android 14/15.

### TODO: multi-generation support

The C source is the same across GKI generations -- only the `Module.symvers` CRCs and kernel headers differ. To support other generations, build against the corresponding ACK branch:

| GKI generation | ACK branch | Devices |
|---|---|---|
| `android13-5.15` | `android13-5.15` | Pixel 7, some 2023 devices |
| `android14-5.15` | `android14-5.15` | Some Samsung on Android 14 |
| `android14-6.1` | `android14-6.1` | **Current build** |
| `android15-6.1` | `android15-6.1` | Pixel 8/9 on Android 15 QPR |
| `android15-6.6` | `android15-6.6` | Future devices |

Each generation needs a separate `.ko`. The build steps are identical -- only the kernel source checkout and `Module.symvers` change. A future CI matrix build could produce all variants from one commit.

## Build

See [BUILDING.md](BUILDING.md) for the full guide (kernel source preparation, toolchain setup, `Module.symvers` generation).

Quick version:

```bash
cd kmod && ./build-zip.sh
```

Requires kernel source for `android14-6.1` + clang cross-compiler.

## Install

1. `adb push vpnhide-kmod.zip /sdcard/Download/`
2. KernelSU-Next manager -> Modules -> Install from storage
3. Reboot

On boot:
- `post-fs-data.sh` runs `insmod` to load the kernel module
- `service.sh` resolves package names from `targets.txt` to UIDs via `pm list packages -U` and writes them to `/proc/vpnhide_targets`

### Target management

**WebUI (recommended):** open the module in KernelSU-Next manager and tap the WebUI entry. Select apps, save. The WebUI writes to **three places** simultaneously:
1. `targets.txt` -- persistent package names (survives module updates)
2. `/proc/vpnhide_targets` -- resolved UIDs for the kernel module
3. `/data/system/vpnhide_uids.txt` -- resolved UIDs for the [lsposed](../lsposed/) module's system_server hooks (live reload via inotify)

All changes apply immediately -- no reboot needed.

**Shell:**
```bash
# Write package names to the persistent config
adb shell su -c 'echo "com.example.targetapp" > /data/adb/vpnhide_kmod/targets.txt'

# Or write UIDs directly to the kernel module
adb shell su -c 'echo 10423 > /proc/vpnhide_targets'
```

## Combined use with system_server hooks

For apps with aggressive anti-tamper SDKs, full VPN hiding requires covering both native and Java API detection paths -- without placing any hooks in the target app's process:

- **vpnhide-kmod** (this module) covers the native side: `ioctl` (`SIOCGIFFLAGS` / `SIOCGIFNAME` / `SIOCGIFCONF`), `getifaddrs()` (via `rtnl_fill_ifinfo`), and `/proc/net/route` (via `fib_route_seq_show`).
- **[lsposed](../lsposed/) system_server hooks** cover the Java API side: `NetworkCapabilities.writeToParcel()`, `NetworkInfo.writeToParcel()`, `LinkProperties.writeToParcel()` -- stripping VPN data before Binder serialization reaches the app.

Together they provide complete VPN hiding without any hooks in the target app's process. The anti-tamper SDK cannot detect either component.

### Setup

1. Install **vpnhide-kmod** as a KSU module (this module).
2. Install **[lsposed](../lsposed/)** as an LSPosed/Vector module and add **"System Framework"** to its scope.
3. Pick target apps in vpnhide-kmod's WebUI -- it manages targets for both the kernel module and the system_server hooks.
4. **Remove** banking apps from lsposed's LSPosed app-process scope (if they were added previously). Only "System Framework" should be in scope for anti-tamper SDK apps -- loading the module into the target app's process will trigger the SDK's anti-tamper detection.

For apps without aggressive anti-tamper SDKs, the standard combination of [lsposed](../lsposed/) (app-process hooks) + [zygisk](../zygisk/) provides more complete Java + native coverage and does not require this kernel module.

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

### rtnl_fill_ifinfo trick

To skip a VPN interface during a netlink dump without corrupting the message stream, the return handler sets the return value to `-EMSGSIZE`. The dump iterator interprets this as "skb too small for this entry" and moves to the next device without adding the current one -- effectively skipping it. The entry is never seen by userspace.

## TODO

- [ ] Multi-GKI-generation CI build (see GKI compatibility section)
- [ ] `/proc/net/tcp`, `tcp6` filtering (`tcp4_seq_show` / `tcp6_seq_show`) -- low priority, only matters for proxy-based VPN clients with open local ports
- [ ] `connect()` filter on localhost proxy ports (`__sys_connect`) -- same caveat as above

## License

GPL-2.0 (required for kernel modules using GPL-only symbols like `register_kretprobe`).
