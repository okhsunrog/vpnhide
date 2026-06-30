#!/usr/bin/env bash
# Boot the vpnhide *KPM* in QEMU (TCG) and validate it hides a fabricated
# VPN interface from a target UID. Mirrors run.sh but for the KernelPatch
# Module backend instead of the .ko:
#
#   1. build vpnhide.kpm (`make kpm`)
#   2. download KernelPatch's kptools-linux + kpimg-linux (cached)
#   3. patch the cached GKI Image TWICE, embedding the .kpm:
#        - phase "notarget": no target UID  -> root must SEE vpn0
#        - phase "target"  : target = uid 0 -> root must NOT see vpn0
#   4. boot each (init-kpm.sh) and diff the per-vector vpn0 counts
#
# Hide vectors PASS iff notarget>0 and target==0. Stable keep vectors must be
# exactly unchanged; aggregate keep vectors must remain positive.
# This needs no /proc (targets come via the embedded extra-args), so it
# validates the inline hooks + per-kver offsets independently of the procfs
# control plane.
#
# Usage:  kmod/test/run-kpm.sh [kmi]      (default: android12-5.10)
set -euo pipefail

KMI="${1:-android12-5.10}"
HERE="$(cd "$(dirname "$0")" && pwd)"
KMOD="$(cd "$HERE/.." && pwd)"
CACHE="$HERE/.cache"
KDIR="$CACHE/$KMI"
# KernelPatch host tool + runtime. Defaults to a per-run download cache, but CI
# points VPNHIDE_KP_BIN at the copies baked into the test image (hermetic — no
# per-run network fetch). If the dir already has both binaries, the fetch below
# is skipped.
KPBIN="${VPNHIDE_KP_BIN:-$CACHE/kp}"

IMAGE="${VPNHIDE_QEMU_IMAGE:-$KDIR/Image}"
KPM="${VPNHIDE_KPM:-$KMOD/vpnhide.kpm}"
ALPINE_VER="3.21.2"
ALPINE_TAR="${VPNHIDE_QEMU_ROOTFS:-$CACHE/alpine-minirootfs-$ALPINE_VER-aarch64.tar.gz}"
ALPINE_URL="https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/aarch64/alpine-minirootfs-$ALPINE_VER-aarch64.tar.gz"
# Prebuilt KernelPatch host tool + generic runtime (no need to build them).
KP_RELEASE="${VPNHIDE_KP_RELEASE:-}" # empty = latest
SKEY="vpnhide-qemu-test"
# QEMU CPU model. `max` is fastest and fine for 5.10+, but older kernels
# (< 5.10, e.g. 5.4 / 4.19 / 4.14) fault on its newer features before the
# console comes up — use `cortex-a57` for those.  Override:
# VPNHIDE_QEMU_CPU=cortex-a57
QEMU_CPU="${VPNHIDE_QEMU_CPU:-max}"

command -v qemu-system-aarch64 >/dev/null || { echo "ERROR: qemu-system-aarch64 not installed"; exit 2; }
[ -f "$IMAGE" ] || { echo "ERROR: kernel missing: $IMAGE — run: $HERE/build-kernel.sh $KMI"; exit 2; }

mkdir -p "$CACHE" "$KPBIN"
[ -f "$ALPINE_TAR" ] || { echo "[run-kpm] fetching Alpine minirootfs…"; curl -fsSL "$ALPINE_URL" -o "$ALPINE_TAR"; }

# --- KernelPatch prebuilts (kptools-linux, kpimg-linux) ----------------------
for a in kptools-linux kpimg-linux; do
	if [ ! -f "$KPBIN/$a" ]; then
		echo "[run-kpm] fetching KernelPatch $a…"
		gh release download ${KP_RELEASE:+"$KP_RELEASE"} --repo bmax121/KernelPatch \
			--pattern "$a" -O "$KPBIN/$a" --clobber
	fi
done
chmod +x "$KPBIN/kptools-linux"

# Fail loudly if the host tool can't run here — the prebuilt kptools-linux is
# linked against a recent glibc (>= 2.38), so an older base image (e.g. Debian
# bookworm = 2.36) trips the dynamic loader. Without this check the patch step
# below fails silently and the harness boots an unpatched kernel, surfacing only
# as the confusing "KPM did not load".
"$KPBIN/kptools-linux" --help >/dev/null 2>&1 || {
	echo "ERROR: $KPBIN/kptools-linux is not runnable in this environment:"
	"$KPBIN/kptools-linux" --help 2>&1 | head -2
	exit 2
}

# --- build the .kpm if not provided -----------------------------------------
if [ ! -f "$KPM" ]; then
	echo "[run-kpm] building vpnhide.kpm…"
	make -C "$KMOD" kpm >/dev/null
fi

# --- static getifaddrs probe (validates the addr-fill hooks) -----------------
# bionic getifaddrs() = RTM_GETLINK + RTM_GETADDR; the `ip addr` vector can't
# isolate the addr path. The probe MUST be a *bionic* binary: it validates how
# *Android's* getifaddrs parses the filtered netlink dump. A glibc build is
# deliberately NOT used — glibc's getifaddrs walks the RTM_GETADDR stream more
# strictly and spins on the filtered output on some kernels (e.g. 5.4); that is
# a glibc parsing quirk, not a product bug (Android is bionic, which passes).
#
# Source order:
#   1. VPNHIDE_GAI_BIN — a prebuilt bionic binary baked into the CI image
#      (built from this exact gai-probe.c with the NDK; see the Dockerfiles).
#   2. an NDK clang on this host (dev machines) — build from source.
#   3. neither — skip the vector (the 8 core vectors still run).
GAI=""
if [ -n "${VPNHIDE_GAI_BIN:-}" ] && [ -x "${VPNHIDE_GAI_BIN:-}" ]; then
	GAI="$VPNHIDE_GAI_BIN"
	echo "[run-kpm] getifaddrs probe: prebuilt bionic ($GAI)"
else
	GAI_CC="${VPNHIDE_GAI_CC:-$(find "$HOME/Android/Sdk/ndk" -type f -path '*/toolchains/llvm/prebuilt/*/bin/aarch64-linux-android*-clang' 2>/dev/null | sort | tail -1 || true)}"
	if [ -n "$GAI_CC" ] && [ -x "$GAI_CC" ]; then
		GAI="$CACHE/gai"
		"$GAI_CC" -static -O2 -o "$GAI" "$HERE/gai-probe.c" 2>/dev/null || GAI=""
	fi
	[ -n "$GAI" ] && echo "[run-kpm] getifaddrs probe built ($(basename "$GAI_CC"))" || \
		echo "[run-kpm] no bionic toolchain/binary — skipping getifaddrs probe (8 core vectors still run)"
fi

# In CI the probe is baked into the image (VPNHIDE_GAI_BIN), so a skip there
# means the baked binary is missing / not executable — a silent addr-fill
# coverage regression that would otherwise still go green. VPNHIDE_GAI_REQUIRED
# (set by the CI jobs) turns that into a hard failure. Locally, with no flag,
# the skip stays soft so the harness still runs without an NDK.
if [ -z "$GAI" ] && [ -n "${VPNHIDE_GAI_REQUIRED:-}" ]; then
	echo "ERROR: getifaddrs probe is required here (VPNHIDE_GAI_REQUIRED set) but" \
	     "unavailable — VPNHIDE_GAI_BIN='${VPNHIDE_GAI_BIN:-}' missing/not executable."
	echo "       Refusing to pass with the addr-fill (inet*_fill_ifaddr) hook unchecked."
	exit 2
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# boot_phase <args> -> echoes the serial log path; sets globals via files
boot_phase() {
	local args="$1" tag="$2"
	local patched="$WORK/Image.$tag" log="$WORK/serial.$tag.log"

	"$KPBIN/kptools-linux" -p -i "$IMAGE" -k "$KPBIN/kpimg-linux" -S "$SKEY" \
		-M "$KPM" -T kpm ${args:+-A "$args"} -o "$patched" >"$WORK/patch.$tag.log" 2>&1

	local rfs="$WORK/rootfs.$tag"
	mkdir -p "$rfs"
	tar xzf "$ALPINE_TAR" -C "$rfs"
	cp "$HERE/init-kpm.sh" "$rfs/init"
	chmod +x "$rfs/init"
	[ -n "$GAI" ] && { cp "$GAI" "$rfs/gai"; chmod +x "$rfs/gai"; }
	( cd "$rfs" && find . | cpio -o -H newc 2>/dev/null | gzip > "$WORK/initramfs.$tag.gz" )

	echo "[run-kpm] $KMI: booting phase '$tag' (args='${args}')…" >&2
	timeout 300 qemu-system-aarch64 \
		-machine virt -cpu "$QEMU_CPU" -accel tcg,thread=multi,tb-size=1024 \
		-smp 4 -m 2G \
		-kernel "$patched" -initrd "$WORK/initramfs.$tag.gz" \
		-append "console=ttyAMA0 rodata=off panic=-1 rdinit=/init" \
		-netdev user,id=n0 -device virtio-net-pci,netdev=n0,romfile= \
		-display none -no-reboot -serial "file:$log" >/dev/null 2>&1 || true
	echo "$log"
}

NT_LOG="$(boot_phase "" notarget)"
TG_LOG="$(boot_phase "0" target)"

vec_count() { grep -oE "VEC $1=[0-9]+" "$2" | head -1 | grep -oE '[0-9]+$' || echo "-1"; }
panic_count() { grep -oE 'PANIC=[0-9]+' "$1" | head -1 | grep -oE '[0-9]+$' || echo "1"; }
kpmload() { grep -q 'KPMLOAD=ok' "$1" && echo ok || echo FAIL; }
keep_mode() {
	case "$1" in
		keep_proc_route_v4|keep_getifaddrs|keep_siocgifconf|keep_dev_ioctl|keep_policy_rule)
			echo exact
			;;
		*)
			echo nonvpn
			;;
	esac
}

echo "------------------------- KPM test output -------------------------"
for log in "$NT_LOG" "$TG_LOG"; do
	grep -E 'KREL|KPMLOAD|KVER|IPROUTE2|VEC |PANIC' "$log" 2>/dev/null | sed "s|^|[$(basename "$log")] |" || true
done
echo "-------------------------------------------------------------------"

[ "$(kpmload "$NT_LOG")" = ok ] || { echo "ERROR: KPM did not load (notarget boot)"; tail -20 "$NT_LOG"; exit 1; }
[ "$(kpmload "$TG_LOG")" = ok ] || { echo "ERROR: KPM did not load (target boot)"; tail -20 "$TG_LOG"; exit 1; }

PASS=0; FAIL=0
for vec in proc_route_v4 getifaddrs proc_route_v6 siocgifconf dev_ioctl netlink_route4 hostroute4 netlink_route6 policy_rule gai_getifaddrs; do
	# The gai_getifaddrs vector only exists when the bionic probe is available
	# (baked VPNHIDE_GAI_BIN, or built from an NDK on this host). If neither is
	# present the probe can't run, so skip the vector instead of failing it.
	if [ "$vec" = gai_getifaddrs ] && [ -z "$GAI" ]; then
		echo "RESULT $vec=SKIP (no bionic getifaddrs probe available)"; continue
	fi
	nt="$(vec_count "$vec" "$NT_LOG")"
	tg="$(vec_count "$vec" "$TG_LOG")"
	if [ "$nt" -gt 0 ] && [ "$tg" -eq 0 ]; then
		echo "RESULT $vec=PASS (notarget=$nt target=$tg mode=hide)"; PASS=$((PASS+1))
	else
		echo "RESULT $vec=FAIL (notarget=$nt target=$tg mode=hide)"; FAIL=$((FAIL+1))
	fi
done

for vec in keep_proc_route_v4 keep_getifaddrs keep_siocgifconf keep_dev_ioctl keep_netlink_route4 keep_policy_rule keep_gai_getifaddrs; do
	if [ "$vec" = keep_gai_getifaddrs ] && [ -z "$GAI" ]; then
		echo "RESULT $vec=SKIP (no bionic getifaddrs probe available)"; continue
	fi
	nt="$(vec_count "$vec" "$NT_LOG")"
	tg="$(vec_count "$vec" "$TG_LOG")"
	mode="$(keep_mode "$vec")"
	if [ "$mode" = exact ]; then
		if [ "$nt" -gt 0 ] && [ "$tg" -eq "$nt" ]; then
			echo "RESULT $vec=PASS (notarget=$nt target=$tg mode=$mode)"; PASS=$((PASS+1))
		else
			echo "RESULT $vec=FAIL (notarget=$nt target=$tg mode=$mode)"; FAIL=$((FAIL+1))
		fi
	elif [ "$nt" -gt 0 ] && [ "$tg" -gt 0 ]; then
		echo "RESULT $vec=PASS (notarget=$nt target=$tg mode=$mode)"; PASS=$((PASS+1))
	else
		echo "RESULT $vec=FAIL (notarget=$nt target=$tg mode=$mode)"; FAIL=$((FAIL+1))
	fi
done

PANIC=$(( $(panic_count "$NT_LOG") + $(panic_count "$TG_LOG") ))
echo "SUMMARY pass=$PASS fail=$FAIL panic=$PANIC"
[ "$FAIL" -eq 0 ] && [ "$PANIC" -eq 0 ] && { echo "[run-kpm] $KMI: PASS"; exit 0; }
echo "[run-kpm] $KMI: FAIL"; exit 1
