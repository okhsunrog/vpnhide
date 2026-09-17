# CI test kernels: trimming the config

Implemented 2026-09-17 (the research behind it is from 2026-09-16). This note is
the record of what the trim does, what it deliberately does **not** do, and the
measurements and dead ends behind both. Read it before adding a line to
[`kmod/test/qemu-trim.config`](../../kmod/test/qemu-trim.config).

## What is built

Every QEMU test kernel is `gki_defconfig` of its branch plus
[`kmod/test/qemu.config`](../../kmod/test/qemu.config) (virtio-net, PL011
console, initramfs, dummy netdev, software PAN, unsigned modules) and now
[`kmod/test/qemu-trim.config`](../../kmod/test/qemu-trim.config), merged second.
The two stay separate on purpose: `qemu.config` is what QEMU needs to boot, the
trim is what the tests do not need, so a boot regression and a build-time
experiment are never the same diff.

Three producers share the recipe, and all three merge both fragments:

- the `ddk-qemu` image bake ([`.github/docker/ddk-qemu/Dockerfile`](../../.github/docker/ddk-qemu/Dockerfile)),
  which also runs the ABI check below before building;
- `kmod/test/build-kernel.sh` and `builtin/test/build-kernel.sh` for local runs;
- the `builtin-qemu` CI job, which re-merges the trim onto the baked tree's
  `.config` per pull request — the only job that exercises the fragment before
  the image is re-baked, because `kmod-qemu` and `kpm-qemu` boot the baked Image.

The legacy 4.9/4.14/4.19/5.4 kernels (`kmod/test/build-source-kernel.sh`) start
from `cuttlefish_defconfig` and are out of scope.

## Measurements

`android14-6.1` at the pinned bake commit `3f3ca1c40482`, the two builds run
back to back in the DDK container on one machine (`make Image -j14`):

| | baseline | trimmed | delta |
|---|---|---|---|
| wall | 512.6 s | 421.6 s | −17.8 % |
| CPU (user) | 6281 s | 5247 s | −16.5 % |
| objects compiled | 2784 | 2446 | −12.1 % |
| `vmlinux` | 354 MB | 41.3 MB | −88 % |
| `Image` | 30.1 MB | 25.8 MB | −14 % |

CPU time is the number to carry over to CI: the runners have fewer cores, so
they see it closer to wall time. Roughly half the saving is the 338 objects that
stop being compiled, the other half is `CONFIG_DEBUG_INFO_NONE` making every
remaining object cheaper — which is also the whole of the `vmlinux` shrink, and
smaller ccache entries against the 10 GB repository cache limit.

Where the objects went (base → trimmed): `fs/nls` stays, but `fs/ext4` 37→0,
`drivers/md` 26→0, `fs/f2fs` 22→0, `drivers/mmc` 22→0, `drivers/scsi` 19→0,
`drivers/ufs` 12→0, `fs/erofs` 12→0, `drivers/usb/gadget` 30→0,
`drivers/nvdimm` 17→0, `drivers/clk` 49→14, `drivers/firmware` 67→45.

## What the trim must never touch

Anything that moves a member of a structure the backends read. The KPM backend
reads those offsets from the hardcoded table in `kmod/kpm/kver_offsets.h` — it
never sees a kernel header — so a shifted offset would make `kpm-qemu` fail or,
worse, pass against a layout no device has. `NETFILTER`/`NF_CONNTRACK`
(`sk_buff._nfct`), `NET_SCHED` (`sk_buff.tc_index`), `BPF_SYSCALL`
(`sock.sk_bpf_storage`), SELinux and the LSM blobs, cgroups, `CFI_CLANG`,
`KASAN_HW_TAGS`, `UBSAN`, `RANDOMIZE_BASE` and `ARM64_SW_TTBR0_PAN` all stay on,
exactly as on a device. LTO stays inherited per KMI.

This is enforced, not just written down. `kmod/test/verify-trim-abi.sh`
configures the same kernel tree twice, with and without the fragment, compiles
`kmod/test/abi/abi-probe.c` against each, and diffs the `pahole` layout of the 49
structures the three backends dereference. It fails on three things: a layout
that moved, a `.config` the two variants agree on (a fragment that did nothing),
and a fragment line the kernel ignored.

It builds one object per variant rather than two kernels, because layouts come
from the headers — `modules_prepare` plus a single `.o` is enough, and the run
takes minutes. Three details were needed to make that object readable:

- debug info is turned back on **through the config** for the probe pass, not
  through `KCFLAGS`: on 6.12 AutoFDO appends `-gmlt` after `KCFLAGS` whenever
  `CONFIG_DEBUG_INFO` is unset, leaving line tables and no types at all;
- `-fno-lto` for the full-LTO KMIs (5.10, 5.15), which would otherwise emit LLVM
  bitcode with no DWARF, and `-fno-sanitize=cfi` with it, because there CFI is
  implemented through LTO and clang rejects one without the other;
- one `pahole` call captured whole per structure. Piping it into `grep -q` kills
  it with SIGPIPE, and under `pipefail` every large structure then looks like a
  missing one.

The guard was checked against a known-bad fragment: adding
`# CONFIG_NF_CONNTRACK is not set` makes it fail and print the vanished
`sk_buff._nfct` (248 → 240 bytes), which is exactly the KPM `skb_len = 112`
assumption breaking.

## The dead end: USB, DRM, media and sound

Those four are the biggest built-in driver trees — about 300 objects, another
11 % of the compile — and none of them can be switched off. `GKI_HACKS_TO_FIX`
`select`s a `GKI_HIDDEN_*` dummy per subsystem so out-of-tree vendor modules
still find their symbols, and `select` beats a fragment:
`# CONFIG_MEDIA_SUPPORT is not set` is ignored outright, while dropping `DRM`,
`SOUND` or `USB_SUPPORT` leaves kconfig in an unmet-dependency state no shipped
kernel is configured in (`SND_JACK=y` with `SND=n`).

Turning the master switch off does not work either, and the ABI check is what
showed why — it was passing on 6.1 and failing on 6.12:

```
-	struct device              dev;                  /*  1376   912 */
+	struct device              dev;                  /*  1376   904 */
```

`arch/arm64/Kconfig` on 6.12 has `select ARCH_HAS_DMA_OPS if (XEN ||
GKI_HACKS_TO_FIX)`, so the master switch is what puts `dma_ops` in `struct
device`; without it that structure loses 8 bytes and the whole tail of `struct
net_device`, which embeds it, shifts. Several symbols it selects are structural
on their own as well: `PAGE_POOL` and `NET_DEVLINK` (`GKI_HIDDEN_NET_CONFIGS`),
`WIRELESS_EXT` (`GKI_LEGACY_WEXT_ALLCONFIG`, `net_device.wireless_handlers`) and
`ARCH_WANTS_DYNAMIC_TASK_STRUCT` (`GKI_DYNAMIC_TASK_STRUCT_SIZE`).

If a future KMI stops routing `DMA_OPS` through `GKI_HACKS_TO_FIX`, this is the
first thing to revisit.

Three smaller lines died the same way and are documented in the fragment so
nobody adds them back: `NLS` (selected by `CONFIG_USB` and `PCI_LABEL`, both of
which stay), `HID` (held up by `BT_HIDP` and a screenful of `=y` leaf drivers on
5.10/5.15; on 6.1 it falls to `=m` on its own once `USB_HID` goes), and
`CLK_TEGRA_BPMP`/`NVME_CORE`, which are selected from above — `ARCH_TEGRA` and
`BLK_DEV_NVME` are the switches that work.

## How it was validated

- `verify-trim-abi.sh` on android12-5.10, android13-5.15, android14-6.1,
  android15-6.6 and android16-6.12, at the commits `qemu-image.yml` pins. The two
  KMIs not checked locally (android13-5.10, android14-5.15) share a kernel
  version with ones that were; the bake runs the check for them.
- A trimmed android14-6.1 kernel booted in QEMU through all three harnesses:
  `kmod/test/run.sh` (37 vectors), `kmod/test/run-kpm.sh` (35 vectors, the
  backend with the hardcoded offsets) and `builtin/test/run.sh` (38 vectors).
  No failures, no panics.
