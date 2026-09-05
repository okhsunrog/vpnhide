#!/usr/bin/env bash
# Boot a GKI kernel with the built-in (CONFIG_VPNHIDE=y) backend in QEMU and run
# the shared vpnhide vector tests inside it. Same harness as kmod/test/run.sh,
# but there is NO .ko: the driver is compiled into the Image, so init.sh only
# verifies /proc/vpnhide_ctl is present (a /vpnhide_backend marker selects that
# path) and then drives the exact same config wire + probes.
#
# Consumes a prebuilt Image from builtin/test/.cache/<kmi>/Image
# (produced by builtin/test/build-kernel.sh).
#
# Usage:  builtin/test/run.sh [kmi]      (default: android14-6.1)
# Exit:   0 = all emitted vectors PASS, no panic; non-zero otherwise.
set -euo pipefail

KMI="${1:-android14-6.1}"
HERE="$(cd "$(dirname "$0")" && pwd)"
KMOD_TEST="$(cd "$HERE/../../kmod/test" && pwd)"
CACHE="$HERE/.cache"
KDIR="$CACHE/$KMI"

IMAGE="${VPNHIDE_QEMU_IMAGE:-$KDIR/Image}"

# GKI Images boot on `-cpu max`; the pre-GKI legacy Images (built from source by
# build-source-kernel.sh) fault on max's newer features before the console —
# boot those with VPNHIDE_QEMU_CPU=cortex-a57. `rodata=off` is harmless on GKI
# and required by some legacy kernels, so it is always passed.
QEMU_CPU="${VPNHIDE_QEMU_CPU:-max}"

ALPINE_VER="3.21.2"
ALPINE_TAR="${VPNHIDE_QEMU_ROOTFS:-$CACHE/alpine-minirootfs-$ALPINE_VER-aarch64.tar.gz}"
ALPINE_URL="https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/aarch64/alpine-minirootfs-$ALPINE_VER-aarch64.tar.gz"

command -v qemu-system-aarch64 >/dev/null || { echo "ERROR: qemu-system-aarch64 not installed"; exit 2; }
[ -f "$IMAGE" ] || { echo "ERROR: kernel missing: $IMAGE"; echo "  run: $HERE/build-kernel.sh $KMI"; exit 2; }

mkdir -p "$CACHE"
[ -f "$ALPINE_TAR" ] || { echo "[run] fetching Alpine minirootfs…"; curl -fsSL "$ALPINE_URL" -o "$ALPINE_TAR"; }

# Native probes (bionic getifaddrs / SIOCGIFCONF / socket-bind) — reuse the
# kmod/test probe sources and the NDK toolchain; SKIP when unavailable.
NDK_CC="$(find "$HOME/Android/Sdk/ndk" -type f -path '*/toolchains/llvm/prebuilt/*/bin/aarch64-linux-android*-clang' 2>/dev/null | sort | tail -1 || true)"
build_probe() { # <src> <out>; echoes out path or empty
	local src="$1" out="$2"
	[ -n "$NDK_CC" ] && [ -x "$NDK_CC" ] || { echo ""; return; }
	"$NDK_CC" -static -O2 -o "$out" "$src" 2>/dev/null && echo "$out" || echo ""
}
GAI="$(build_probe "$KMOD_TEST/gai-probe.c" "$CACHE/gai")"
IFC="$(build_probe "$KMOD_TEST/ifconf-probe.c" "$CACHE/ifconf")"
BIND="$(build_probe "$KMOD_TEST/bind-probe.c" "$CACHE/bind-probe")"
IOC="$(build_probe "$KMOD_TEST/iface-ioctl-probe.c" "$CACHE/iface-ioctl")"
[ -n "$NDK_CC" ] && echo "[run] native probes built ($(basename "$NDK_CC"))" || \
	echo "[run] no NDK toolchain — native probes SKIP (core /proc + iproute2 vectors still run)"

# 32-bit (armv7) bind probe: exercises the compat setsockopt path. On pre-5.9
# kernels that is a separate entry (compat_sock_setsockopt), so this is what
# catches a bind hook patched only at the 64-bit __sys_setsockopt. Same source,
# just the armv7 toolchain; SKIP if it is unavailable.
NDK_CC32="$(find "$HOME/Android/Sdk/ndk" -type f -path '*/toolchains/llvm/prebuilt/*/bin/armv7a-linux-androideabi*-clang' 2>/dev/null | sort | tail -1 || true)"
BIND32=""
if [ -n "$NDK_CC32" ] && [ -x "$NDK_CC32" ]; then
	"$NDK_CC32" -static -O2 -o "$CACHE/bind-probe32" "$KMOD_TEST/bind-probe-compat.c" 2>/dev/null &&
		BIND32="$CACHE/bind-probe32"
fi
[ -n "$BIND32" ] && echo "[run] compat (armv7) bind probe built" || \
	echo "[run] no armv7 NDK — compat bind vector SKIP"

WORK="$(mktemp -d)"
if [ -n "${VPNHIDE_QEMU_KEEP_WORK:-}" ]; then
	echo "[run] preserving QEMU work directory: $WORK"
else
	trap 'rm -rf "$WORK"' EXIT
fi
RFS="$WORK/rootfs"
mkdir -p "$RFS"
tar xzf "$ALPINE_TAR" -C "$RFS"
cp "$KMOD_TEST/init.sh" "$RFS/init"
chmod +x "$RFS/init"
printf 'builtin\n' > "$RFS/vpnhide_backend"   # selects the in-tree path in init.sh
[ -n "$GAI" ]  && { cp "$GAI"  "$RFS/gai";        chmod +x "$RFS/gai"; }
[ -n "$IFC" ]  && { cp "$IFC"  "$RFS/ifconf";     chmod +x "$RFS/ifconf"; }
[ -n "$BIND" ] && { cp "$BIND" "$RFS/bind-probe"; chmod +x "$RFS/bind-probe"; }
[ -n "$BIND32" ] && { cp "$BIND32" "$RFS/bind-probe32"; chmod +x "$RFS/bind-probe32"; }
[ -n "$IOC" ] && { cp "$IOC" "$RFS/iface-ioctl"; chmod +x "$RFS/iface-ioctl"; }
( cd "$RFS" && find . | cpio -o -H newc 2>/dev/null | gzip > "$WORK/initramfs.cpio.gz" )

LOG="$WORK/serial.log"
echo "[run] $KMI: booting built-in $(basename "$IMAGE") in QEMU (TCG, no KVM)…"
timeout 300 qemu-system-aarch64 \
	-machine virt -cpu "$QEMU_CPU" -accel tcg,thread=multi,tb-size=1024 \
	-smp 4 -m 2G \
	-kernel "$IMAGE" -initrd "$WORK/initramfs.cpio.gz" \
	-append "console=ttyAMA0 rodata=off panic=-1 rdinit=/init" \
	-netdev user,id=n0 -device virtio-net-pci,netdev=n0,romfile= \
	-display none -no-reboot -serial "file:$LOG" >/dev/null 2>&1 || true

echo "------------------------- test output -------------------------"
if ! sed -n '/VPNHIDE-QEMU-TEST START/,/VPNHIDE-QEMU-TEST END/p' "$LOG" |
	grep -E 'KREL|IPROUTE2|INSMOD|REGISTERED|RESULT|PANIC|SUMMARY'; then
	echo "ERROR: no test output — kernel did not boot or init failed"
	echo "--- last 30 serial lines ---"; tail -30 "$LOG"
	exit 1
fi
echo "---------------------------------------------------------------"

summary="$(grep -oE 'SUMMARY pass=[0-9]+ fail=[0-9]+ panic=[0-9]+ registered=[0-9]+' "$LOG" | tail -1 || true)"
[ -n "$summary" ] || { echo "ERROR: VM did not reach SUMMARY (boot failure)"; exit 1; }
fail="$(sed -E 's/.*fail=([0-9]+).*/\1/' <<<"$summary")"
panic="$(sed -E 's/.*panic=([0-9]+).*/\1/' <<<"$summary")"

if [ "$fail" -eq 0 ] && [ "$panic" -eq 0 ]; then
	echo "[run] $KMI: PASS ($summary)"
	exit 0
fi
echo "[run] $KMI: FAIL ($summary)"
exit 1
