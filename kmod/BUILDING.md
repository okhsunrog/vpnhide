# Building vpnhide-kmod

Most users should download pre-built modules from [Releases](https://github.com/okhsunrog/vpnhide/releases) — builds are provided for all supported GKI generations. This guide is for contributors or users who need to build from source.

## Quick build with DDK (recommended)

The easiest way to build is using the same DDK Docker images that CI uses. No kernel source clone, no toolchain setup.

```bash
# Pick your GKI generation (see "Identifying your GKI generation" below)
KMI=android14-6.1

# Build the kernel module
docker run --rm -v $(pwd)/kmod:/work \
    ghcr.io/ylarod/ddk-min:${KMI}-20260313 sh -c "
    CLANG=\$(echo /opt/ddk/clang/clang-r*/bin) && \
    make -C /opt/ddk/kdir/${KMI} M=/work \
        ARCH=arm64 LLVM=1 LLVM_IAS=1 \
        CC=\$CLANG/clang LD=\$CLANG/ld.lld \
        AR=\$CLANG/llvm-ar NM=\$CLANG/llvm-nm \
        OBJCOPY=\$CLANG/llvm-objcopy \
        OBJDUMP=\$CLANG/llvm-objdump \
        STRIP=\$CLANG/llvm-strip \
        CROSS_COMPILE=aarch64-linux-gnu- \
        modules"

# Package as KSU module
cp kmod/vpnhide_kmod.ko kmod/module/
(cd kmod/module && zip -qr ../../vpnhide-kmod.zip .)
```

## Local build with kernel source

If you prefer building against a local kernel source tree (e.g. for development or debugging), use the Makefile with `direnv`:

```bash
cd kmod/
cp .env.example .env
# Edit .env with paths to your kernel source and clang toolchain
direnv allow
make
./build-zip.sh
```

See `.env.example` for the required variables. You need a prepared kernel source tree with headers and `Module.symvers`.

## Identifying your GKI generation

```bash
adb shell uname -r
```

The output looks like `6.1.75-android14-11-g...` — the generation is `android14-6.1`.

> **Note:** the `android14` part is NOT your Android version — it's the kernel generation. All Pixels from 6 to 9a share the same `android14-6.1` kernel. Pixel 10 series moves to `android16-6.12`.

| `uname -r` pattern | GKI generation |
|---|---|
| `5.10.xxx-android12-...` | android12-5.10 |
| `5.10.xxx-android13-...` | android13-5.10 |
| `5.15.xxx-android13-...` | android13-5.15 |
| `5.15.xxx-android14-...` | android14-5.15 |
| `6.1.xxx-android14-...` | android14-6.1 |
| `6.6.xxx-android15-...` | android15-6.6 |
| `6.12.xxx-android16-...` | android16-6.12 |

## Install and test

```bash
adb push vpnhide-kmod.zip /sdcard/Download/
# Install via KernelSU-Next manager -> Modules -> Install from storage
# Reboot
```

Verify after reboot:

```bash
adb shell "su -c 'lsmod | grep vpnhide'"
adb shell "su -c 'dmesg | grep vpnhide'"
adb shell "su -c 'cat /proc/vpnhide_targets'"
```

## Troubleshooting

**`insmod: Exec format error`** — symvers CRC mismatch. Use the DDK build (matched symvers).

**`insmod: File exists`** — module already loaded. `rmmod vpnhide_kmod` first.

**kretprobe not firing** — check `dmesg | grep vpnhide` for registration messages and `/proc/vpnhide_targets` for correct UIDs. Target app UIDs change on reinstall — re-resolve via the VPN Hide app.
