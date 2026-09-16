# CI test kernels: trimming the config

Working note, 2026-09-16. Research only; nothing here is implemented yet. It is
the brief for the change, with the measurements it rests on and the safety rule
that must not be broken.

## What is built today

Every QEMU test kernel is `gki_defconfig` of its branch plus
[`kmod/test/qemu.config`](../../kmod/test/qemu.config) (virtio-net, PL011
console, initramfs, dummy netdev, software PAN, unsigned modules). Three
producers share that recipe:

- the `ddk-qemu` image bake ([`.github/docker/ddk-qemu/Dockerfile`](../../.github/docker/ddk-qemu/Dockerfile),
  run by `qemu-image.yml` monthly and when the Dockerfile changes), which bakes
  every GKI KMI, including android12-5.10 with full LTO;
- `kmod/test/build-kernel.sh` and `builtin/test/build-kernel.sh` for local runs;
- the `builtin-qemu` CI job, which re-runs `make Image` in the baked tree per
  pull request after `integrate.py` patched it (three KMIs: 6.1, 6.6, 6.12).

The legacy 4.9/4.14/4.19/5.4 kernels (`kmod/test/build-source-kernel.sh`) start
from `cuttlefish_defconfig` and are out of scope here.

## Measurements

From the `builtin-qemu (android14-6.1)` job of the last CI run on `main`
(2026-09-15, before ccache):

| Phase | Time |
|---|---|
| compile: 2783 objects, none of them modules (`make Image` builds no `obj-m`) | 12.3 min |
| link vmlinux + Image | about 0.5 min |
| QEMU boot + test | 20 s |

What the build log shows are built-in drivers, not modules: `gki_defconfig`
compiles subsystems into the Image that `qemu -machine virt` never has.
Objects per directory: `arch/arm64` 177, `drivers/usb` 101, `net/netfilter` 94,
`net/ipv4` 91, `drivers/gpu` 80, `net/ipv6` 71, `drivers/firmware` 67,
`drivers/base` 58, `drivers/media` 55, `fs/nls` 52, `drivers/clk` 49,
`drivers/pci` 46, `drivers/hid` 46, `kernel/bpf` 41, `drivers/tty` 38,
`fs/ext4` 37, `net/sched` 35, `drivers/md` 26, `sound/core` 25. Attributing log
timestamp deltas to the directory of the object being compiled (rough under
`-j`), `drivers/*` together is about a third of the compile, `net/*` about 15 %,
`arch/arm64` 7 %.

What the tests actually use (`kmod/test/run.sh`, `builtin/test/run.sh`,
`kmod/test/run-kpm.sh`, `kmod/test/init.sh`): virtio-net-pci and the PL011
console, initramfs with proc/sysfs/devtmpfs, a dummy netdev, kprobes, module
loading. No `-drive` is ever passed and nothing is mounted from a disk.

Debug settings in the defconfigs: 6.1+ has `DEBUG_INFO_DWARF4=y` and
`DEBUG_INFO_BTF=y`; 5.10 has `DEBUG_INFO=y` plus `LTO_CLANG_FULL=y`. All have
`CFI_CLANG`, `KASAN_HW_TAGS`, `UBSAN` (trap, local bounds).

## What can go, and what must stay

Safe to disable (drivers and filesystems do not touch the structures the hooks
read): USB including gadget, DRM/GPU, MEDIA, SOUND, HID, vendor CLK and
firmware drivers (keep PSCI and EFI), MD/DM, SCSI, NLS, EXT4/F2FS, the block
side of virtio, staging. Expected: 25-35 % less compile work.

Also safe and worth it: `CONFIG_DEBUG_INFO_NONE=y` for test kernels. Neither
kprobes, nor KernelPatch, nor the harness use DWARF or BTF. It changes no
structure, cuts compile by an estimated 10-15 %, shortens the link and makes the
ccache objects smaller.

Must stay, because each one changes the layout of a structure the hooks or
KernelPatch read: `NETFILTER` and `NF_CONNTRACK` (`sk_buff._nfct`), `NET_SCHED`
(`sk_buff.tc_index`), `BPF_SYSCALL` (`sock.sk_bpf_storage`), SELinux and the
LSM blobs (`sock.sk_security`, `file`, `inode`), cgroups, `CFI_CLANG`,
`KASAN_HW_TAGS`, `UBSAN`, `RANDOMIZE_BASE`, `ARM64_SW_TTBR0_PAN`. The KPM relies
on per-family struct layouts (see the "KPM offset mismatch" history), so a shifted
offset would make `kpm-qemu` either fail or, worse, pass against a layout no
device has.

Leave full LTO on 5.10 alone. `qemu.config` explains why the LTO mode is
inherited per KMI: on 5.10 CFI requires LTO, and LTO's symbol mangling caught
real bugs. It only costs the monthly bake, not pull requests.

## Where the gain lands

With ccache (`builtin-qemu` compiles through it since the CI refactor), an
incremental pull-request run recompiles only what the integrator patched and
relinks, so trimming matters for the cold runs (after an image rebuild or a
cache eviction), for the bake itself, and for image size. It also lowers the
per-KMI ccache footprint against the 10 GB repository cache limit.

## Brief for the change

1. Add `kmod/test/qemu-trim.config` (`# CONFIG_X is not set` for the subsystems
   above, plus `CONFIG_DEBUG_INFO_NONE=y`) and merge it after `qemu.config` in
   the Dockerfile, `kmod/test/build-kernel.sh` and `builtin/test/build-kernel.sh`.
   Keep the two fragments separate: `qemu.config` is what QEMU needs, the trim
   is what the tests do not.
2. Safety net before merging: build 6.1 once with and once without the trim and
   compare, with `pahole`, the sizes and member offsets of `struct sock`,
   `sk_buff`, `net_device`, `socket`, `task_struct`, `file`, `inode`, `ifreq`
   and whatever else `kmod/shared` reads. Any difference means the fragment
   disabled something structural. Turn the comparison into a script the bake
   runs, so a future defconfig change cannot silently move an offset either.
3. Rebuild the images, run the three QEMU matrices (kmod, kpm, builtin) on them,
   then measure a cold `builtin-qemu` against the 12.3-minute baseline above.
