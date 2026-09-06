#!/usr/bin/env bash
# Boot the vpnhide *KPM* in QEMU (TCG) and validate it hides a fabricated
# VPN interface from a target UID. Mirrors run.sh but for the KernelPatch
# Module backend instead of the .ko:
#
#   1. build vpnhide.kpm (`make kpm`)
#   2. download KernelPatch's kptools-linux + kpimg-linux (cached)
#   3. patch the cached GKI Image TWICE, embedding the .kpm:
#        - phase "notarget": no target UID      -> app uid 10000 must SEE vpn0
#        - phase "target"  : control-v2 config targets uid 10000 -> hidden
#   4. boot each (init-kpm.sh) and diff the per-vector vpn0 counts
#
# Hide vectors PASS iff notarget>0 and target==0. Stable keep vectors must be
# exactly unchanged; aggregate keep vectors must remain positive.
# This needs no /proc (the config snapshot comes via embedded extra-args), so it
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
# (< 5.10, e.g. 5.4 / 4.19 / 4.14 / 4.9) fault on its newer features before the
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

# --- static SIOCGIFCONF probe (visible entries + stale tail) ----------------
IFC=""
if [ -n "${VPNHIDE_IFC_BIN:-}" ] && [ -x "${VPNHIDE_IFC_BIN:-}" ]; then
	IFC="$VPNHIDE_IFC_BIN"
	echo "[run-kpm] ifconf probe: prebuilt ($IFC)"
else
	IFC_CC="${VPNHIDE_IFC_CC:-$(find "$HOME/Android/Sdk/ndk" -type f -path '*/toolchains/llvm/prebuilt/*/bin/aarch64-linux-android*-clang' 2>/dev/null | sort | tail -1 || true)}"
	if [ -n "$IFC_CC" ] && [ -x "$IFC_CC" ]; then
		IFC="$CACHE/ifconf"
		"$IFC_CC" -static -O2 -Wall -Wextra -Werror \
			-o "$IFC" "$HERE/ifconf-probe.c" 2>/dev/null || IFC=""
	fi
	[ -n "$IFC" ] && echo "[run-kpm] ifconf probe built ($(basename "$IFC_CC"))" || \
		echo "[run-kpm] no bionic toolchain/binary — skipping SIOCGIFCONF tail probe"
fi

if [ -z "$IFC" ] && [ -n "${VPNHIDE_IFC_REQUIRED:-}" ]; then
	echo "ERROR: ifconf probe is required here (VPNHIDE_IFC_REQUIRED set) but unavailable."
	exit 2
fi

# --- state-aware socket binding probe ----------------------------------------
BIND=""
if [ -n "${VPNHIDE_BIND_BIN:-}" ] && [ -x "${VPNHIDE_BIND_BIN:-}" ]; then
	BIND="$VPNHIDE_BIND_BIN"
	echo "[run-kpm] socket bind probe: prebuilt ($BIND)"
else
	BIND_CC="${VPNHIDE_BIND_CC:-$(find "$HOME/Android/Sdk/ndk" -type f -path '*/toolchains/llvm/prebuilt/*/bin/aarch64-linux-android*-clang' 2>/dev/null | sort | tail -1 || true)}"
	if [ -n "$BIND_CC" ] && [ -x "$BIND_CC" ]; then
		BIND="$CACHE/bind-probe"
		"$BIND_CC" -static -O2 -Wall -Wextra -Werror \
			-o "$BIND" "$HERE/bind-probe.c" 2>/dev/null || BIND=""
	fi
	[ -n "$BIND" ] && echo "[run-kpm] socket bind probe built ($(basename "$BIND_CC"))" || \
		echo "[run-kpm] no bionic toolchain/binary — skipping socket bind vectors"
fi

if [ -z "$BIND" ] && [ -n "${VPNHIDE_BIND_REQUIRED:-}" ]; then
	echo "ERROR: socket bind probe is required here (VPNHIDE_BIND_REQUIRED set) but unavailable."
	exit 2
fi

# --- by-name SIOCGIFHWADDR / SIOCGIFADDR probe ------------------------------
# Isolates the get-by-name ioctls (dev_get_mac_address / devinet_ioctl) that do
# not flow through the dev_ioctl dispatcher the `dev_ioctl` shell vector uses —
# in particular SIOCGIFADDR (devinet_ioctl), the path the KPM #1 fix added.
IOC=""
if [ -n "${VPNHIDE_IOC_BIN:-}" ] && [ -x "${VPNHIDE_IOC_BIN:-}" ]; then
	IOC="$VPNHIDE_IOC_BIN"
	echo "[run-kpm] iface-ioctl probe: prebuilt ($IOC)"
else
	IOC_CC="${VPNHIDE_IOC_CC:-$(find "$HOME/Android/Sdk/ndk" -type f -path '*/toolchains/llvm/prebuilt/*/bin/aarch64-linux-android*-clang' 2>/dev/null | sort | tail -1 || true)}"
	if [ -n "$IOC_CC" ] && [ -x "$IOC_CC" ]; then
		IOC="$CACHE/iface-ioctl"
		"$IOC_CC" -static -O2 -o "$IOC" "$HERE/iface-ioctl-probe.c" 2>/dev/null || IOC=""
	fi
	[ -n "$IOC" ] && echo "[run-kpm] iface-ioctl probe built ($(basename "$IOC_CC"))" || \
		echo "[run-kpm] no bionic toolchain/binary — skipping by-name ioctl vectors"
fi

if [ -z "$IOC" ] && [ -n "${VPNHIDE_IOC_REQUIRED:-}" ]; then
	echo "ERROR: iface-ioctl probe is required here (VPNHIDE_IOC_REQUIRED set) but unavailable."
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
	[ -n "$IFC" ] && { cp "$IFC" "$rfs/ifconf"; chmod +x "$rfs/ifconf"; }
	[ -n "$BIND" ] && { cp "$BIND" "$rfs/bind-probe"; chmod +x "$rfs/bind-probe"; }
	[ -n "$IOC" ] && { cp "$IOC" "$rfs/iface-ioctl"; chmod +x "$rfs/iface-ioctl"; }
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
# Both shell vectors and the state-aware bind probe use app uid 10000 (0x2710).
# The harness passes the same control-v2 snapshot as runtime ctl0; load args are
# only the transport needed before the in-VM userspace client is available.
TARGET_CONFIG=$'vpnhide 2 config\ndebug 0\ntargets a0003ff 2710\nend 1\n'
TG_LOG="$(boot_phase "$TARGET_CONFIG" target)"

vec_count() { grep -oE "VEC $1=[0-9]+" "$2" | head -1 | grep -oE '[0-9]+$' || echo "-1"; }
ifc_count() { grep -oE "$1=[0-9]+" "$2" | head -1 | grep -oE '[0-9]+$' || echo "-1"; }
panic_count() { grep -oE 'PANIC=[0-9]+' "$1" | head -1 | grep -oE '[0-9]+$' || echo "1"; }
kpmload() { grep -q 'KPMLOAD=ok' "$1" && echo ok || echo FAIL; }
keep_mode() {
	case "$1" in
		keep_proc_route_v4|keep_getifaddrs|keep_siocgifconf|keep_dev_ioctl|keep_policy_rule|keep_sysfs_readdir|keep_proc_sys_readdir)
			echo exact
			;;
		*)
			echo nonvpn
			;;
	esac
}

echo "------------------------- KPM test output -------------------------"
for log in "$NT_LOG" "$TG_LOG"; do
	grep -E 'KREL|KPMLOAD|KVER|KPMLOG|IPROUTE2|TARGET_USER|VEC |BIND_|PANIC' "$log" 2>/dev/null | sed "s|^|[$(basename "$log")] |" || true
done
echo "-------------------------------------------------------------------"

[ "$(kpmload "$NT_LOG")" = ok ] || { echo "ERROR: KPM did not load (notarget boot)"; tail -20 "$NT_LOG"; exit 1; }
[ "$(kpmload "$TG_LOG")" = ok ] || { echo "ERROR: KPM did not load (target boot)"; tail -20 "$TG_LOG"; exit 1; }
grep -q 'TARGET_USER=ok' "$NT_LOG" || { echo "ERROR: app test user unavailable (notarget boot)"; exit 1; }
grep -q 'TARGET_USER=ok' "$TG_LOG" || { echo "ERROR: app test user unavailable (target boot)"; exit 1; }

PASS=0; NATIVE=0; NA=0; SKIP=0; FAIL=0

bind_field() {
	local key="$1" log="$2"
	sed -n "s/^${key}=//p" "$log" | tr -d '\r' | head -1
}

check_bind_pair() {
	local vec="$1" prefix="$2"
	local nt_errno nt_state tg_errno tg_state
	nt_errno="$(bind_field "${prefix}_ERRNO" "$NT_LOG")"
	nt_state="$(bind_field "${prefix}_STATE" "$NT_LOG")"
	tg_errno="$(bind_field "${prefix}_ERRNO" "$TG_LOG")"
	tg_state="$(bind_field "${prefix}_STATE" "$TG_LOG")"

	if [ -z "$nt_errno" ] || [ -z "$nt_state" ] || [ -z "$tg_errno" ] || [ -z "$tg_state" ]; then
		echo "RESULT $vec=SKIP (socket bind probe unavailable)"
		SKIP=$((SKIP+1))
		return
	fi
	# SO_BINDTODEVICE needs CAP_NET_RAW on these legacy kernels. Upstream added
	# SO_BINDTOIFINDEX in 5.7, but Android common backported it to 5.4 with the
	# same capability gate; 4.x returns ENOPROTOOPT instead. Report those proven
	# kernel properties separately: they close the vector but do not exercise the
	# KPM bind hook. SKIP is reserved for a probe that did not run.
	if [ "$nt_errno" -ne 0 ] &&
		{ [ "$nt_state" -eq 0 ] || [ "$nt_state" -eq "$((-nt_errno))" ]; }; then
		if [ "$tg_errno" -eq "$nt_errno" ] && [ "$tg_state" -eq "$nt_state" ]; then
			if [ "$prefix" = BIND_INDEX ] && [ "$nt_errno" -eq 92 ]; then
				echo "RESULT $vec=NOT-APPLICABLE (kernel has no SO_BINDTOIFINDEX)"
				NA=$((NA+1))
				return
			fi
			if [ "$nt_errno" -eq 1 ] &&
				{ [ "$prefix" = BIND_NAME_RAW ] || [ "$prefix" = BIND_NAME_NUL ] ||
				  [ "$prefix" = BIND_INDEX ]; }; then
				echo "RESULT $vec=NATIVE-COVERED (CAP_NET_RAW gate denied and socket stayed unbound)"
				NATIVE=$((NATIVE+1))
				return
			fi
		fi
		echo "RESULT $vec=FAIL (unexpected native result: nt_errno=$nt_errno nt_state=$nt_state tg_errno=$tg_errno tg_state=$tg_state)"
		FAIL=$((FAIL+1))
		return
	fi
	if [ "$nt_errno" -eq 0 ] && [ "$nt_state" -eq 1 ] && \
		[ "$tg_errno" -eq 19 ] && [ "$tg_state" -eq 0 ]; then
		echo "RESULT $vec=PASS (notarget=bound target=ENODEV+unbound)"
		PASS=$((PASS+1))
	else
		echo "RESULT $vec=FAIL (nt_errno=$nt_errno nt_state=$nt_state tg_errno=$tg_errno tg_state=$tg_state)"
		FAIL=$((FAIL+1))
	fi
}

check_bind_pair bind_device_raw BIND_NAME_RAW
check_bind_pair bind_device_nul BIND_NAME_NUL
check_bind_pair bind_ifindex BIND_INDEX

# By-name SIOCGIFHWADDR / SIOCGIFADDR (dev_get_mac_address / devinet_ioctl): the
# target must get the native ENODEV(19) a genuinely-absent name returns, while a
# non-target still reads the value (non-ENODEV). Same criterion as the .ko
# harness (init.sh check_iface_ioctl). Both are unprivileged GETs, so there is no
# #3-style cap-ordering to reconcile.
check_ioctl_pair() {
	local vec="$1" key="$2" nt tg
	nt="$(bind_field "$key" "$NT_LOG")"
	tg="$(bind_field "$key" "$TG_LOG")"
	if [ -z "$nt" ] || [ -z "$tg" ]; then
		echo "RESULT $vec=SKIP (iface-ioctl probe unavailable)"
		SKIP=$((SKIP+1))
		return
	fi
	if [ "$tg" -eq 19 ] && [ "$nt" -ne 19 ]; then
		echo "RESULT $vec=PASS (target ENODEV, non-target visible: nt=$nt)"
		PASS=$((PASS+1))
	else
		echo "RESULT $vec=FAIL (nt_errno=$nt tg_errno=$tg)"
		FAIL=$((FAIL+1))
	fi
}
check_ioctl_pair ioctl_hwaddr HWADDR_ERRNO
check_ioctl_pair ioctl_addr ADDR_ERRNO

nt_bad_errno="$(bind_field BIND_BADPTR_ERRNO "$NT_LOG")"
nt_bad_state="$(bind_field BIND_BADPTR_STATE "$NT_LOG")"
tg_bad_errno="$(bind_field BIND_BADPTR_ERRNO "$TG_LOG")"
tg_bad_state="$(bind_field BIND_BADPTR_STATE "$TG_LOG")"
if [ -z "$nt_bad_errno" ] || [ -z "$nt_bad_state" ] || \
	[ -z "$tg_bad_errno" ] || [ -z "$tg_bad_state" ]; then
	echo "RESULT bind_bad_pointer=SKIP (socket bind probe unavailable)"
	SKIP=$((SKIP+1))
elif [ "$nt_bad_errno" -ne 0 ] && [ "$nt_bad_state" -eq 0 ] && \
	[ "$tg_bad_errno" -ne 0 ] && [ "$tg_bad_state" -eq 0 ]; then
	echo "RESULT bind_bad_pointer=PASS (safe rejection+unbound)"; PASS=$((PASS+1))
else
	echo "RESULT bind_bad_pointer=FAIL (nt_errno=$nt_bad_errno nt_state=$nt_bad_state tg_errno=$tg_bad_errno tg_state=$tg_bad_state)"; FAIL=$((FAIL+1))
fi

nt_badlen_errno="$(bind_field BIND_BADLEN_ERRNO "$NT_LOG")"
nt_badlen_state="$(bind_field BIND_BADLEN_STATE "$NT_LOG")"
tg_badlen_errno="$(bind_field BIND_BADLEN_ERRNO "$TG_LOG")"
tg_badlen_state="$(bind_field BIND_BADLEN_STATE "$TG_LOG")"
if [ -z "$nt_badlen_errno" ] || [ -z "$nt_badlen_state" ] || \
	[ -z "$tg_badlen_errno" ] || [ -z "$tg_badlen_state" ]; then
	echo "RESULT bind_bad_length=SKIP (socket bind probe unavailable)"
	SKIP=$((SKIP+1))
elif [ "$nt_badlen_errno" -ne 0 ] && [ "$nt_badlen_state" -eq 0 ] && \
	[ "$tg_badlen_errno" -eq "$nt_badlen_errno" ] && [ "$tg_badlen_state" -eq 0 ]; then
	echo "RESULT bind_bad_length=PASS (identical native rejection+unbound)"; PASS=$((PASS+1))
else
	echo "RESULT bind_bad_length=FAIL (nt_errno=$nt_badlen_errno nt_state=$nt_badlen_state tg_errno=$tg_badlen_errno tg_state=$tg_badlen_state)"; FAIL=$((FAIL+1))
fi

nt_keep_errno="$(bind_field BIND_KEEP_ERRNO "$NT_LOG")"
nt_keep_state="$(bind_field BIND_KEEP_STATE "$NT_LOG")"
tg_keep_errno="$(bind_field BIND_KEEP_ERRNO "$TG_LOG")"
tg_keep_state="$(bind_field BIND_KEEP_STATE "$TG_LOG")"
if [ -z "$nt_keep_errno" ] || [ -z "$tg_keep_errno" ]; then
	echo "RESULT keep_bind_device=SKIP (socket bind probe unavailable)"
	SKIP=$((SKIP+1))
elif [ "$nt_keep_errno" -ne 0 ] && [ "${nt_keep_state:--1}" -eq 0 ] && \
	[ "$nt_keep_errno" -eq 1 ] && [ "$tg_keep_errno" -eq "$nt_keep_errno" ] && \
	[ "${tg_keep_state:--1}" -eq "$nt_keep_state" ]; then
	echo "RESULT keep_bind_device=NATIVE-COVERED (CAP_NET_RAW behavior preserved)"
	NATIVE=$((NATIVE+1))
elif [ "$nt_keep_errno" -eq 0 ] && [ "$nt_keep_state" -eq 1 ] && \
	[ "$tg_keep_errno" -eq 0 ] && [ "$tg_keep_state" -eq 1 ]; then
	echo "RESULT keep_bind_device=PASS (physical bind preserved)"; PASS=$((PASS+1))
else
	echo "RESULT keep_bind_device=FAIL (nt_errno=$nt_keep_errno nt_state=$nt_keep_state tg_errno=$tg_keep_errno tg_state=$tg_keep_state)"; FAIL=$((FAIL+1))
fi

nt_absent_errno="$(bind_field BIND_ABSENT_ERRNO "$NT_LOG")"
nt_absent_state="$(bind_field BIND_ABSENT_STATE "$NT_LOG")"
if [ -z "$nt_absent_errno" ] || [ -z "$nt_absent_state" ]; then
	echo "RESULT bind_absent_name=SKIP (socket bind probe unavailable)"
	SKIP=$((SKIP+1))
elif [ "$nt_absent_errno" -ne 0 ] && [ "$nt_absent_state" -eq 0 ]; then
	# The measured "no such interface" answer for this kernel family: EPERM
	# where CAP_NET_RAW is checked before the name is parsed, ENODEV where the
	# name is resolved first.  Recorded because the zygisk backend mirrors it
	# at runtime (hidden_bind_errno) instead of deriving it from the version.
	echo "RESULT bind_absent_name=PASS (errno=$nt_absent_errno unbound)"; PASS=$((PASS+1))
else
	echo "RESULT bind_absent_name=FAIL (errno=$nt_absent_errno state=$nt_absent_state)"; FAIL=$((FAIL+1))
fi

if [ -z "$IFC" ]; then
	echo "RESULT ifconf_tail=SKIP (no ifconf probe available)"
	SKIP=$((SKIP+1))
else
	nt_ifc_vpn="$(ifc_count IFCONF_FILL_VPN "$NT_LOG")"
	tg_ifc_vpn="$(ifc_count IFCONF_FILL_VPN "$TG_LOG")"
	nt_ifc_tail="$(ifc_count IFCONF_TAIL "$NT_LOG")"
	tg_ifc_tail="$(ifc_count IFCONF_TAIL "$TG_LOG")"
	if [ "$nt_ifc_vpn" -gt 0 ] && [ "$tg_ifc_vpn" -eq 0 ] && \
		[ "$nt_ifc_tail" -eq 0 ] && [ "$tg_ifc_tail" -eq 0 ]; then
		echo "RESULT ifconf_tail=PASS (nt_vpn=$nt_ifc_vpn tg_vpn=$tg_ifc_vpn nt_tail=$nt_ifc_tail tg_tail=$tg_ifc_tail)"; PASS=$((PASS+1))
	else
		echo "RESULT ifconf_tail=FAIL (nt_vpn=$nt_ifc_vpn tg_vpn=$tg_ifc_vpn nt_tail=$nt_ifc_tail tg_tail=$tg_ifc_tail)"; FAIL=$((FAIL+1))
	fi
fi

for vec in proc_route_v4 getifaddrs proc_route_v6 siocgifconf dev_ioctl netlink_route4 hostroute4 netlink_route6 hostroute6 policy_rule sysfs_stat sysfs_open sysfs_readdir proc_sys_stat proc_sys_readdir gai_getifaddrs; do
	# The gai_getifaddrs vector only exists when the bionic probe is available
	# (baked VPNHIDE_GAI_BIN, or built from an NDK on this host). If neither is
	# present the probe can't run, so skip the vector instead of failing it.
	if [ "$vec" = gai_getifaddrs ] && [ -z "$GAI" ]; then
		echo "RESULT $vec=SKIP (no bionic getifaddrs probe available)"; SKIP=$((SKIP+1)); continue
	fi
	# init-kpm.sh emits `VEC <name>=SKIP` for a vector that doesn't apply to the
	# running kernel (e.g. the host-route on non-GKI <5.6 kernels).
	if grep -q "VEC $vec=SKIP" "$NT_LOG"; then
		echo "RESULT $vec=NOT-APPLICABLE (not supported on this kernel)"; NA=$((NA+1)); continue
	fi
	nt="$(vec_count "$vec" "$NT_LOG")"
	tg="$(vec_count "$vec" "$TG_LOG")"
	if [ "$nt" -gt 0 ] && [ "$tg" -eq 0 ]; then
		echo "RESULT $vec=PASS (notarget=$nt target=$tg mode=hide)"; PASS=$((PASS+1))
	else
		echo "RESULT $vec=FAIL (notarget=$nt target=$tg mode=hide)"; FAIL=$((FAIL+1))
	fi
done

for vec in keep_proc_route_v4 keep_getifaddrs keep_siocgifconf keep_dev_ioctl keep_netlink_route4 keep_policy_rule keep_sysfs_readdir keep_proc_sys_readdir keep_gai_getifaddrs; do
	if [ "$vec" = keep_gai_getifaddrs ] && [ -z "$GAI" ]; then
		echo "RESULT $vec=SKIP (no bionic getifaddrs probe available)"; SKIP=$((SKIP+1)); continue
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
echo "SUMMARY pass=$PASS native-covered=$NATIVE not-applicable=$NA skip=$SKIP fail=$FAIL panic=$PANIC"
[ "$FAIL" -eq 0 ] && [ "$PANIC" -eq 0 ] && { echo "[run-kpm] $KMI: PASS"; exit 0; }
echo "[run-kpm] $KMI: FAIL"; exit 1
