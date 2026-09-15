# kmod QEMU test harness

Boots a reference kernel under `qemu-system-aarch64` and runs the vpnhide
kernel module against fabricated VPN state. It compares the enabled vectors for
a *target* UID and a *non-target* UID; optional native probes are reported as
skipped when their binaries are unavailable.

This exists because compilation and source-level signature checks are **not
sufficient** for this module: it reads syscall/netlink arguments straight out
of `pt_regs` by register index, so correctness depends on the *actual* argument
→ register mapping in the built kernel — which a compiler can change (notably
for `static`, directly-called functions). Only running the module on the real
kernel proves the registers are right. (This is exactly how the `rt_fill_info`
register bug was found — see Design decisions.)

## What it checks (and what it can't)

Per kernel version, the harness validates:

- the module **loads** (symbol resolution against that kernel),
- hook registration is reported for diagnosis,
- each detection vector is actually **filtered** for a target UID (A/B: a
  non-target sees the fabricated `vpn0`, a target does not),
- non-VPN entries such as `eth0`, the main policy rule, and bionic
  `getifaddrs()` non-vpn rows remain visible to the target UID,
- stable non-VPN counters (`/proc/net/route`, `ip addr`, `ifconfig`, direct
  `dev_ioctl`, and the main policy rule) are **exactly unchanged** between the
  non-target and target passes; aggregate counters (`ip route show table all`
  and bionic `getifaddrs()` non-vpn rows) must stay positive,
- raw `SO_BINDTODEVICE` (with and without a trailing NUL) and
  `SO_BINDTOIFINDEX` calls bind `vpn0` for a non-target but return `ENODEV` and
  leave the socket unbound for a target; binding physical `eth0` still works,
- malformed option pointers and lengths preserve native errors and leave the
  socket unbound instead of being dereferenced or fingerprinting the hook,
- optional VFS concealment makes `vpn0` absent from stat/open/readdir probes
  under sysfs and `/proc/sys/net`, while keeping physical `eth0` entries,
- ordinary `rmmod` is refused and leaves the module/control plane active — the
  entry-kprobe replacement text intentionally remains resident until reboot,
- **no kernel panic** across the whole hook set.

Vectors exercised (`init.sh`):

| Vector | Trigger | Hook |
|---|---|---|
| getifaddrs | `ip addr show` | `rtnl_fill_ifinfo` + `inet*_fill_ifaddr` |
| bionic getifaddrs | `/gai` static NDK probe | `rtnl_fill_ifinfo` + `inet*_fill_ifaddr` |
| SIOCGIFCONF | `ifconfig -a` + `/ifconf` oversized-buffer probe | `sock_ioctl` |
| dev ioctl by name | `ifconfig vpn0` | `dev_ioctl` |
| /proc/net/route | `cat /proc/net/route` | `fib_route_seq_show` |
| /proc/net/ipv6_route | `cat /proc/net/ipv6_route` | `ipv6_route_seq_show` |
| netlink route dump v4 | `ip route show table all` | `fib_dump_info` |
| netlink route dump v6 | `ip -6 route show table all` | `rt6_fill_node` |
| policy rules | `ip rule show` | `fib_nl_fill_rule` |
| socket bind by name/index | `/bind-probe` static NDK raw-syscall probe | `socket_bind_interface` |
| sysfs stat/open/readdir | `/sys/class/net/vpn0` and its directory | `filesystem_iface_paths` |
| proc-sys stat/readdir | `/proc/sys/net/ipv4/{conf,neigh}/vpn0` | `filesystem_iface_paths` |

**Limits:** GitHub/QEMU runners have no KVM, so the VM runs under TCG (software
emulation) — correct, just slow. The test kernel is *our* `kernel/common` build
with `qemu.config` merged, not a byte-identical vendor GKI release, so it does
not cover vendor patches. A device smoke-test is still the final word — but the
gate is far tighter than "it compiled".

## Usage

```sh
# 1. Build a bootable kernel + a matching module for a KMI (slow, ~15-40 min;
#    clones kernel/common, builds Image with the DDK container's clang, builds
#    the module against that tree, caches under .cache/<kmi>/).
kmod/test/build-kernel.sh android12-5.10

# 2. Boot it in QEMU and run the vector tests.
kmod/test/run.sh android12-5.10
```

`run.sh` exits 0 when every emitted vector passes and there was no panic.
Optional probes can be made mandatory with their `*_REQUIRED` variables.
Artifacts
(kernels, modules, the Alpine rootfs) are cached under `.cache/` (gitignored).
`run.sh` honors `VPNHIDE_QEMU_IMAGE` / `VPNHIDE_QEMU_ROOTFS` / `VPNHIDE_QEMU_KO`
to use prebuilt artifacts (CI) instead of the local cache. It also honors
`VPNHIDE_GAI_BIN` for a prebuilt static bionic `getifaddrs()` probe and
`VPNHIDE_GAI_REQUIRED=1` to fail instead of silently skipping that native probe.
`VPNHIDE_IFC_BIN` / `VPNHIDE_IFC_REQUIRED=1` control the `SIOCGIFCONF`
size-query and stale-tail probe, and `VPNHIDE_BIND_BIN` /
`VPNHIDE_BIND_REQUIRED=1` provide the equivalent control for the state-aware
socket-bind probe.

Requirements: `docker` (for build-kernel.sh), `qemu-system-aarch64`, `cpio`,
`curl`.

## How CI uses it

Building a virtio GKI kernel takes ~15-40 min, so it must not run per-PR:

- **`.github/workflows/qemu-image.yml`** bakes per-KMI images
  `ghcr.io/<owner>/vpnhide/ddk-qemu:<kmi>` = `FROM ddk-min:<kmi>` + qemu + the
  built kernel `Image` + **its build tree** (`/opt/qemu/linux`, for module
  builds) + the Alpine rootfs. Matrix over the 7 KMIs; runs only on
  `qemu.config`/Dockerfile changes, monthly, or manual dispatch. Full-LTO
  generations are heavy — the workflow frees disk + adds swap.
- **`ci.yml` `kmod-qemu` job** (per KMI) boots the baked kernel and runs the
  vectors. It builds the module **against the baked kernel tree**
  (`VPNHIDE_QEMU_KSRC`), not the GKI kdir — see Design decisions. The image is
  repo-owned (private), so the job pulls it with `credentials:` and a
  lowercased name from the `setup` job output, like the other image jobs.

## KPM backend harness (`run-kpm.sh`)

`run-kpm.sh` is the sibling for the [KPM backend](../kpm/): instead of
`insmod`-ing a `.ko`, it patches a bootable `Image` with KernelPatch
(`kptools-linux` + `kpimg-linux`, fetched from the KernelPatch releases),
embeds `vpnhide.kpm`, and boots the patched kernel under the same QEMU/TCG
setup. The KPM loads at boot (KernelPatch hijacks `paging_init`), so there
is no insmod and no `/proc` dependency — the same control-v2 snapshot used by
runtime ctl0 is passed via embedded extra-args (`kptools -A`). The A/B is done across **two boots**
(no target → app UID 10000 sees `vpn0`; target=10000 → it doesn't), driven by
`init-kpm.sh`. The 10 enumeration hooks, socket-bind state checks, and optional
sysfs/proc-sys stat/open/readdir checks run with no panic across the CI reference set — 4.9, 4.14, 4.19, 5.4, 5.10,
5.15, 6.1, 6.6, 6.12 (the modern GKI ones via
the DDK `Image`, the legacy AOSP ones via a from-source `Image` passed with
`VPNHIDE_QEMU_IMAGE=`). Legacy kernels that natively require `CAP_NET_RAW` for
the first bind report `NATIVE-COVERED` (including Android common 5.4's
backported `SO_BINDTOIFINDEX`); 4.x kernels without that option report
`NOT-APPLICABLE`. `SKIP` is reserved for a probe that did not run, so neither
result pretends that the KPM bind hook fired.

```sh
make -C kmod kpm                 # build vpnhide.kpm
kmod/test/run-kpm.sh android12-5.10
```

Two knobs the KPM harness needs that the `.ko` one doesn't:

- **`VPNHIDE_QEMU_CPU`** — defaults to `max`, but kernels older than 5.10
  (4.9/4.14/4.19/5.4) fault on `max`'s newer CPU features before the console comes
  up, so they need `VPNHIDE_QEMU_CPU=cortex-a57`. KernelPatch also writes its
  restore region into kernel text, so the boot args carry `rodata=off` (some
  from-source layouts otherwise put that page read-only → early abort).
- **PAN is enforced on every kernel.** `qemu.config` sets
  `CONFIG_ARM64_SW_TTBR0_PAN=y` so a hook that dereferences a *userspace*
  pointer as kernel memory **panics in the test** rather than silently passing.
  QEMU emulates whatever the chosen `-cpu` advertises: `-cpu max` advertises
  hardware PAN (FEAT_PAN) and the GKI kernels run on it, but the <5.10 kernels
  must run on `-cpu cortex-a57` (ARMv8.0, *no* PAN feature) — so on the exact
  CPU where the bug lives there is no hardware PAN to emulate. Software PAN
  (`SW_TTBR0_PAN`, the kernel's own TTBR0-switching fallback) enforces the
  boundary independent of the CPU model, so it works under both `-cpu` choices.
  This is how a real bug shipped: `dev_ioctl`'s ifreq arg is a `__user` pointer
  on <5.5 kernels, and reading it directly only faulted on real PAN hardware (a
  Pixel 4a on 4.14) until SW PAN was turned on here — now the buggy build
  panics on the `target` boot of the 4.14 run, and the fixed build is clean.

## Design decisions

Each of these was forced by a concrete failure; they are easy to "simplify"
back into a regression.

1. **Build the module against the baked kernel tree, not the GKI kdir.**
   A module must match the kernel it runs on (CFI tags, vermagic, the
   `struct kretprobe` layout). The kdir (what ships to devices) and our
   separately-built QEMU kernel are *different builds*; on `android16-6.12`
   their configs diverge enough to be fatal: with CFI on, insmod hits a CFI
   type-id mismatch panic; with CFI off, a `struct kretprobe` mismatch crashes
   `pre_handler_kretprobe` (NULL deref). 5.10/6.1/6.6 only matched by luck.
   So the image keeps the kernel tree and the module is built against it.

2. **Make the baked tree a complete external-module build environment.**
   `make Image` alone is not enough. Also run `make modules_prepare` (generates
   `scripts/module.lds`, else modfinal: "No rule to make target …ko") and
   `cp vmlinux.symvers Module.symvers` (else modpost reports every kernel symbol
   undefined — `make Image` emits only `vmlinux.symvers`, and our module
   references only vmlinux symbols). `.cmd` files are pruned to shrink the image.

3. **Do not force LTO — inherit it from each KMI's `gki_defconfig`.**
   That is the device-faithful setting and it varies: android12/13-5.10 + 5.15
   ship **full LTO + CFI** (5.10's CFI *requires* LTO), android14-6.1+ ship
   **LTO_NONE + CFI via KCFI** (confirmed against the shipped Pixel 8 Pro
   factory image: `CONFIG_LTO_NONE=y`, `CONFIG_CFI_CLANG=y`). Forcing one mode
   breaks both fidelity and the build (e.g. forcing `LTO_NONE` on 5.10 drops CFI
   as an unmet dependency). LTO barely affects the harness anyway — every hooked
   function is global, address-taken, or too large to inline.

4. **CFI stays on.** With the module built against the baked tree (decision 1)
   it gets matching KCFI and loads fine. Disabling CFI was a wrong turn — it
   only traded the CFI panic for the kretprobe-struct panic.

5. **Disable BTF (`CONFIG_DEBUG_INFO_BTF` off).** The BTFIDS step
   (`resolve_btfids` over vmlinux) fails `Error 255` on some GKI tips with the
   DDK container's pahole. BTF isn't needed for these tests, so dropping it
   removes a flaky build step and lightens the build.

6. **`rt_fill_info` is intentionally not hooked.** It is a `static`,
   directly-called function with no stable arg→register ABI (verified in QEMU:
   `regs[3]` held `table_id`, not the `rtable*`). Route *enumeration* (what
   detection apps use) goes through the global `fib_dump_info`; single lookups
   respect the caller's routing, which is physical under split-tunnel. See
   `docs/notes/ROADMAP.md` for the `rtnl_unicast`-based alternative if ever needed.

7. **iproute2 over QEMU user-net.** `init.sh` apk-adds iproute2 at boot (the
   runner has internet via QEMU's slirp NAT). Prebaking it into the rootfs is a
   possible optimization if the Alpine CDN proves flaky.
