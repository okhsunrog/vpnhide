#!/usr/bin/env bash
# Boot a GKI kernel for <kmi> in QEMU (TCG) and run the vpnhide kmod vector
# tests inside it. Consumes a prebuilt kernel Image + module .ko from the
# per-KMI cache (produced by build-kernel.sh); assembles a throwaway Alpine
# initramfs around init.sh, boots, and parses the serial console for the
# RESULT/SUMMARY lines.
#
# Usage:  kmod/test/run.sh [kmi]      (default: android12-5.10)
# Exit:   0 = all emitted vectors PASS, no panic; non-zero otherwise.
set -euo pipefail

KMI="${1:-android12-5.10}"
HERE="$(cd "$(dirname "$0")" && pwd)"
CACHE="$HERE/.cache"
KDIR="$CACHE/$KMI"

# Inputs default to the local cache (populated by build-kernel.sh), but each
# can be overridden by env so CI can point at artifacts baked into the
# ddk-qemu image / downloaded from the kmod build job:
#   VPNHIDE_QEMU_IMAGE  - kernel Image (baked in the image)
#   VPNHIDE_QEMU_KO     - module .ko   (from the kmod build artifact)
#   VPNHIDE_QEMU_ROOTFS - Alpine minirootfs tarball (baked in the image)
#   VPNHIDE_GAI_BIN     - optional prebuilt bionic getifaddrs probe
#   VPNHIDE_BIND_BIN    - optional prebuilt raw-syscall socket bind probe
IMAGE="${VPNHIDE_QEMU_IMAGE:-$KDIR/Image}"
KO="${VPNHIDE_QEMU_KO:-$KDIR/vpnhide_kmod.ko}"

ALPINE_VER="3.21.2"
ALPINE_TAR="${VPNHIDE_QEMU_ROOTFS:-$CACHE/alpine-minirootfs-$ALPINE_VER-aarch64.tar.gz}"
ALPINE_URL="https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/aarch64/alpine-minirootfs-$ALPINE_VER-aarch64.tar.gz"

command -v qemu-system-aarch64 >/dev/null || { echo "ERROR: qemu-system-aarch64 not installed"; exit 2; }
[ -f "$IMAGE" ] || { echo "ERROR: kernel missing: $IMAGE"; echo "  run: $HERE/build-kernel.sh $KMI"; exit 2; }
[ -f "$KO" ]    || { echo "ERROR: module missing: $KO";   echo "  run: $HERE/build-kernel.sh $KMI"; exit 2; }

mkdir -p "$CACHE"
[ -f "$ALPINE_TAR" ] || { echo "[run] fetching Alpine minirootfs…"; curl -fsSL "$ALPINE_URL" -o "$ALPINE_TAR"; }

# --- static getifaddrs probe (validates the addr-fill hooks) -----------------
# Must be bionic, not glibc, because Android apps use bionic getifaddrs().
GAI=""
if [ -n "${VPNHIDE_GAI_BIN:-}" ] && [ -x "${VPNHIDE_GAI_BIN:-}" ]; then
	GAI="$VPNHIDE_GAI_BIN"
	echo "[run] getifaddrs probe: prebuilt bionic ($GAI)"
else
	GAI_CC="${VPNHIDE_GAI_CC:-$(find "$HOME/Android/Sdk/ndk" -type f -path '*/toolchains/llvm/prebuilt/*/bin/aarch64-linux-android*-clang' 2>/dev/null | sort | tail -1 || true)}"
	if [ -n "$GAI_CC" ] && [ -x "$GAI_CC" ]; then
		GAI="$CACHE/gai"
		"$GAI_CC" -static -O2 -o "$GAI" "$HERE/gai-probe.c" 2>/dev/null || GAI=""
	fi
	[ -n "$GAI" ] && echo "[run] getifaddrs probe built ($(basename "$GAI_CC"))" || \
		echo "[run] no bionic toolchain/binary — skipping native getifaddrs probe (core vectors still run)"
fi

if [ -z "$GAI" ] && [ -n "${VPNHIDE_GAI_REQUIRED:-}" ]; then
	echo "ERROR: getifaddrs probe is required here (VPNHIDE_GAI_REQUIRED set) but" \
	     "unavailable — VPNHIDE_GAI_BIN='${VPNHIDE_GAI_BIN:-}' missing/not executable."
	echo "       Refusing to pass with the addr-fill (inet*_fill_ifaddr) hook unchecked."
	exit 2
fi

# --- static SIOCGIFCONF probe (validates size query + stale tail) ------------
# Reuses the same bionic toolchain as the getifaddrs probe (any libc would do
# for this one, but keep it consistent). Built only if a toolchain is around;
# the harness SKIPs the probe when it's missing.
IFC=""
if [ -n "${VPNHIDE_IFC_BIN:-}" ] && [ -x "${VPNHIDE_IFC_BIN:-}" ]; then
	IFC="$VPNHIDE_IFC_BIN"
	echo "[run] ifconf probe: prebuilt ($IFC)"
else
	IFC_CC="${VPNHIDE_GAI_CC:-$(find "$HOME/Android/Sdk/ndk" -type f -path '*/toolchains/llvm/prebuilt/*/bin/aarch64-linux-android*-clang' 2>/dev/null | sort | tail -1 || true)}"
	if [ -n "$IFC_CC" ] && [ -x "$IFC_CC" ]; then
		IFC="$CACHE/ifconf"
		"$IFC_CC" -static -O2 -o "$IFC" "$HERE/ifconf-probe.c" 2>/dev/null || IFC=""
	fi
	[ -n "$IFC" ] && echo "[run] ifconf probe built ($(basename "$IFC_CC"))" || \
		echo "[run] no bionic toolchain/binary — skipping SIOCGIFCONF probe"
fi

if [ -z "$IFC" ] && [ -n "${VPNHIDE_IFC_REQUIRED:-}" ]; then
	echo "ERROR: ifconf probe is required here (VPNHIDE_IFC_REQUIRED set) but" \
	     "unavailable. Refusing to pass with SIOCGIFCONF size/tail behavior unchecked."
	exit 2
fi

# --- state-aware socket binding probe ----------------------------------------
# This must be a native arm64 binary because it issues setsockopt with an inline
# svc and shares the resulting socket across target/non-target child processes.
BIND=""
if [ -n "${VPNHIDE_BIND_BIN:-}" ] && [ -x "${VPNHIDE_BIND_BIN:-}" ]; then
	BIND="$VPNHIDE_BIND_BIN"
	echo "[run] socket bind probe: prebuilt ($BIND)"
else
	BIND_CC="${VPNHIDE_BIND_CC:-$(find "$HOME/Android/Sdk/ndk" -type f -path '*/toolchains/llvm/prebuilt/*/bin/aarch64-linux-android*-clang' 2>/dev/null | sort | tail -1 || true)}"
	if [ -n "$BIND_CC" ] && [ -x "$BIND_CC" ]; then
		BIND="$CACHE/bind-probe"
		"$BIND_CC" -static -O2 -Wall -Wextra -Werror \
			-o "$BIND" "$HERE/bind-probe.c" 2>/dev/null || BIND=""
	fi
	[ -n "$BIND" ] && echo "[run] socket bind probe built ($(basename "$BIND_CC"))" || \
		echo "[run] no bionic toolchain/binary — skipping socket bind vectors"
fi

# By-name SIOCGIFHWADDR / SIOCGIFADDR probe (dev_ioctl / devinet_ioctl vectors).
IOC=""
IOC_CC="${VPNHIDE_BIND_CC:-$(find "$HOME/Android/Sdk/ndk" -type f -path '*/toolchains/llvm/prebuilt/*/bin/aarch64-linux-android*-clang' 2>/dev/null | sort | tail -1 || true)}"
if [ -n "$IOC_CC" ] && [ -x "$IOC_CC" ]; then
	IOC="$CACHE/iface-ioctl"
	"$IOC_CC" -static -O2 -o "$IOC" "$HERE/iface-ioctl-probe.c" 2>/dev/null || IOC=""
fi
[ -n "$IOC" ] && echo "[run] iface-ioctl probe built" || \
	echo "[run] no toolchain — skipping SIOCGIFHWADDR/ADDR vectors"

if [ -z "$BIND" ] && [ -n "${VPNHIDE_BIND_REQUIRED:-}" ]; then
	echo "ERROR: socket bind probe is required here (VPNHIDE_BIND_REQUIRED set) but unavailable."
	exit 2
fi

WORK="$(mktemp -d)"
if [ -n "${VPNHIDE_QEMU_KEEP_WORK:-}" ]; then
	echo "[run] preserving QEMU work directory: $WORK"
else
	trap 'rm -rf "$WORK"' EXIT
fi
RFS="$WORK/rootfs"
mkdir -p "$RFS"
tar xzf "$ALPINE_TAR" -C "$RFS"
cp "$KO" "$RFS/vpnhide_kmod.ko"
cp "$HERE/init.sh" "$RFS/init"
chmod +x "$RFS/init"
[ -n "$GAI" ] && { cp "$GAI" "$RFS/gai"; chmod +x "$RFS/gai"; }
[ -n "$IFC" ] && { cp "$IFC" "$RFS/ifconf"; chmod +x "$RFS/ifconf"; }
[ -n "$BIND" ] && { cp "$BIND" "$RFS/bind-probe"; chmod +x "$RFS/bind-probe"; }
[ -n "$IOC" ] && { cp "$IOC" "$RFS/iface-ioctl"; chmod +x "$RFS/iface-ioctl"; }
( cd "$RFS" && find . | cpio -o -H newc 2>/dev/null | gzip > "$WORK/initramfs.cpio.gz" )

LOG="$WORK/serial.log"
echo "[run] $KMI: booting $(basename "$IMAGE") in QEMU (TCG, no KVM)…"
# romfile= disables the virtio-net PCI option ROM (iPXE) so we don't need the
# ipxe-qemu package — only matters for PXE boot, which we never do; networking
# (for apk in the VM) still works.
timeout 300 qemu-system-aarch64 \
	-machine virt -cpu max -accel tcg,thread=multi,tb-size=1024 \
	-smp 4 -m 2G \
	-kernel "$IMAGE" -initrd "$WORK/initramfs.cpio.gz" \
	-append "console=ttyAMA0 panic=-1 rdinit=/init" \
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
[ -n "$summary" ] || { echo "ERROR: VM did not reach SUMMARY (boot/insmod failure)"; exit 1; }
fail="$(sed -E 's/.*fail=([0-9]+).*/\1/' <<<"$summary")"
panic="$(sed -E 's/.*panic=([0-9]+).*/\1/' <<<"$summary")"

if [ "$fail" -eq 0 ] && [ "$panic" -eq 0 ]; then
	echo "[run] $KMI: PASS ($summary)"
	exit 0
fi
echo "[run] $KMI: FAIL ($summary)"
exit 1
