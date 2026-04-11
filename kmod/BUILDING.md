# Building vpnhide-kmod for a new device

This guide walks through building the kernel module for any Android
device with a GKI 2.0 kernel (Android 12+, kernel 5.10+). The process
was developed and tested on Pixel 8 Pro (android14-6.1) and applies
identically to any GKI generation — only the kernel branch and
Module.symvers change.

Total time: ~15 minutes once toolchain is set up.
Download size: ~500 MB (shallow kernel clone) + ~100 MB (toolchain if
not already available).

## Prerequisites

- A rooted Android device with KernelSU or Magisk (for adb root shell)
- Linux host (Arch, Ubuntu, Debian — anything with make, clang, git)
- `adb` connected to the device
- `modprobe` on the host (for extracting CRCs from .ko files)
- [`direnv`](https://direnv.net/) (for automatic env var loading)

## Step 1: Identify the GKI generation

```bash
adb shell uname -r
```

Example outputs and what they mean:

| `uname -r` output | GKI generation | Kernel branch |
|---|---|---|
| `5.10.xxx-android13-4-g...` | android13-5.10 | `android13-5.10` |
| `5.15.xxx-android14-6-g...` | android14-5.15 | `android14-5.15` |
| `6.1.xxx-android14-11-g...` | android14-6.1 | `android14-6.1` |
| `6.1.xxx-android15-8-g...` | android15-6.1 | `android15-6.1` |
| `6.6.xxx-android15-...` | android15-6.6 | `android15-6.6` |

The `androidXX` part is the GKI generation (kernel branch name), NOT
the Android version running on the device. A Pixel 7 Pro running
Android 16 still has an `android13-5.10` kernel because the generation
is frozen at manufacturing time.

## Step 2: Clone the full Pixel kernel tree

Google ships a single `android14-6.1` kernel covering all Pixels from 6 to
9a. The full tree includes kernel sources, prebuilt toolchain, device trees,
and all out-of-tree Google modules — everything needed with no extra downloads:

```bash
mkdir ~/pixel-kernel && cd ~/pixel-kernel

repo init --depth=1 \
    -u https://android.googlesource.com/kernel/manifest \
    -b android-gs-shusky-6.1-android16

repo sync -c --no-tags -j$(nproc)
```

Then build once to produce all artifacts:

```bash
./build_shusky.sh   # or build_raviole.sh etc. — same kernel image for all
```

Set an env var pointing to the tree root — subsequent steps reference it:

```bash
export PIXEL_KERNEL_TREE=~/pixel-kernel
```

### Useful paths within the tree

| Path | Purpose |
|---|---|
| `$PIXEL_KERNEL_TREE/aosp/` | Full GKI kernel source tree — browse subsystems, understand exported APIs, read in-tree driver implementations |
| `$PIXEL_KERNEL_TREE/out/shusky/dist/kernel-headers.tar.gz` | Kernel headers for building against (extract and use as `KDIR`) |
| `$PIXEL_KERNEL_TREE/out/shusky/dist/vmlinux.symvers` | Symbol versions for `modpost` — use instead of extracting CRCs from device .ko files |
| `$PIXEL_KERNEL_TREE/out/shusky/dist/System.map` | Symbol addresses for debugging |
| `$PIXEL_KERNEL_TREE/private/google-modules/` | Reference implementations of out-of-tree modules (Kconfig, Makefile patterns) |
| `$PIXEL_KERNEL_TREE/prebuilts/clang/host/linux-x86/clang-r487747c/bin` | Bundled clang 17.0.2 — exact compiler used to build the kernel |

`raviole/dist/` and `shusky/dist/` contain identical `vmlinux.symvers` and
`Image` since it's the same kernel build — use either.

### Prepare kernel headers for module builds

```bash
mkdir -p ~/pixel-kernel-headers
tar -xzf $PIXEL_KERNEL_TREE/out/shusky/dist/kernel-headers.tar.gz \
    -C ~/pixel-kernel-headers
cp $PIXEL_KERNEL_TREE/out/shusky/dist/vmlinux.symvers \
    ~/pixel-kernel-headers/Module.symvers
```

Then skip to step 3.

### Alternative: shallow clone (non-Pixel or different GKI generation)

If you don't need the full tree, clone just the kernel source:

```bash
BRANCH="android13-5.10"  # ← replace with your generation from step 1

git clone --depth=1 -b $BRANCH \
    https://android.googlesource.com/kernel/common \
    ~/kernel-source
```

You'll then need to prepare it manually — see [Preparing a standalone kernel source](#preparing-a-standalone-kernel-source) at the end.

---

## Step 3: Configure .env

The build system uses `direnv` to load `KERNEL_SRC` and `CLANG_DIR` from
a `.env` file. Copy the example and fill in your paths:

```bash
cd kmod/
cp .env.example .env
```

Edit `.env`:

```bash
# Pixel kernel tree approach (after extracting headers above):
KERNEL_SRC=~/pixel-kernel-headers
CLANG_DIR=~/pixel-kernel/prebuilts/clang/host/linux-x86/clang-r487747c/bin

# Or standalone kernel source approach:
# KERNEL_SRC=~/kernel-source
# CLANG_DIR=/path/to/clang/bin
```

Allow direnv to load it:

```bash
direnv allow
```

From now on, entering the `kmod/` directory automatically exports
`KERNEL_SRC` and `CLANG_DIR`. The Makefile reads both from the environment.

## Step 4: Build and package

```bash
make            # builds vpnhide_kmod.ko
./build-zip.sh  # builds .ko (if needed) + packages vpnhide-kmod.zip
```

## Step 5: Install and test

```bash
adb push vpnhide-kmod.zip /sdcard/Download/
# Install via KernelSU-Next manager -> Modules -> Install from storage
# Reboot
```

After reboot, verify:

```bash
# Module loaded?
adb shell "su -c 'lsmod | grep vpnhide'"

# kretprobes registered?
adb shell "su -c 'dmesg | grep vpnhide'"

# UIDs loaded?
adb shell "su -c 'cat /proc/vpnhide_targets'"
```

Pick target apps via the WebUI in KernelSU-Next manager.

---

## Preparing a standalone kernel source

If you used the shallow clone (non-Pixel path), you need to prepare the
kernel source before building. If you used the Pixel kernel tree with
extracted headers, skip this section entirely.

### Pull .config from the device

```bash
adb shell "su -c 'gzip -d < /proc/config.gz'" > ~/kernel-source/.config
```

If `/proc/config.gz` doesn't exist, use the GKI defconfig:

```bash
cd ~/kernel-source
make ARCH=arm64 LLVM=1 CC=$CLANG_DIR/clang gki_defconfig
```

### Generate Module.symvers from device .ko files

The Module.symvers file contains CRC checksums for every exported
kernel symbol. These must match the running kernel exactly
(CONFIG_MODVERSIONS):

```bash
# Pull all vendor modules from the device
mkdir -p /tmp/device-modules
adb shell "su -c 'ls /vendor/lib/modules/*.ko'" | tr -d '\r' | while read ko; do
    adb shell "su -c 'cat $ko'" > "/tmp/device-modules/$(basename $ko)"
done

# Extract CRCs from all modules and build Module.symvers
for ko in /tmp/device-modules/*.ko; do
    modprobe --dump-modversions "$ko" 2>/dev/null
done | sort -u -k2 | \
    awk '{printf "%s\t%s\tvmlinux\tEXPORT_SYMBOL\t\n", $1, $2}' \
    > ~/kernel-source/Module.symvers

echo "Generated Module.symvers with $(wc -l < ~/kernel-source/Module.symvers) symbols"
```

Expect 3000-5000 symbols. If you get 0, check that `modprobe` is
installed (`apt install kmod` or `pacman -S kmod`).

Alternative: if the device's ROM has a `-kernels` repo on GitHub
(e.g. `crdroidandroid/android_device_google_shusky-kernels`), you can
download the .ko files from there instead of pulling from device.

### Prepare headers

```bash
cd ~/kernel-source

# Create empty ABI symbol list (GKI build expects it)
touch abi_symbollist.raw

# Generate headers
make ARCH=arm64 LLVM=1 LLVM_IAS=1 \
    CC=$CLANG_DIR/clang LD=$CLANG_DIR/ld.lld AR=$CLANG_DIR/llvm-ar \
    NM=$CLANG_DIR/llvm-nm OBJCOPY=$CLANG_DIR/llvm-objcopy \
    OBJDUMP=$CLANG_DIR/llvm-objdump STRIP=$CLANG_DIR/llvm-strip \
    CROSS_COMPILE=aarch64-linux-gnu- \
    olddefconfig prepare
```

**Common issue:** `make prepare` may fail on `tools/bpf/resolve_btfids`
due to host clang version mismatch. This is fine — the module can
still build without BTF. Ignore this error.

### Set UTS_RELEASE (vermagic)

KernelSU bypasses vermagic checks, so any value works if using KSU.
For Magisk or manual insmod, the vermagic must match `uname -r`.

```bash
cd ~/kernel-source

# For KernelSU users (any placeholder works):
echo '#define UTS_RELEASE "6.1.0-vpnhide"' > include/generated/utsrelease.h
echo -n "6.1.0-vpnhide" > include/config/kernel.release

# For Magisk users (must match exactly):
KVER="$(adb shell uname -r | tr -d '\r')"
echo "#define UTS_RELEASE \"$KVER\"" > include/generated/utsrelease.h
echo -n "$KVER" > include/config/kernel.release
```

Then set `KERNEL_SRC=~/kernel-source` in your `.env` and proceed to step 4.

---

## Troubleshooting

**`insmod: Exec format error`**
- Vermagic mismatch (Magisk doesn't bypass it). Set UTS_RELEASE to
  exact `uname -r` value.
- Or Module.symvers CRCs don't match — re-extract from device .ko files.

**`insmod: File exists`**
- Module already loaded. `rmmod vpnhide_kmod` first.

**`modprobe --dump-modversions: no output`**
- .ko files might be stripped. Try pulling from a different path
  (`/vendor/lib/modules/`, `/system/lib/modules/`,
  `/lib/modules/$(uname -r)/`).

**`make prepare` fails on resolve_btfids**
- Ignore — BTF is optional. The module builds without it.

**No `/proc/config.gz`**
- Use `make gki_defconfig` instead.

**kretprobe not firing (ioctl not filtered)**
- Check `dmesg | grep vpnhide` for registration messages.
- Check `/proc/vpnhide_targets` has the right UIDs.
- The target app's UID changes on reinstall — re-resolve via WebUI.
