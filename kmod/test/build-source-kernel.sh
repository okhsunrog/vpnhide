#!/usr/bin/env bash
# Build a QEMU-bootable, from-source kernel for the KPM harness's pre-GKI /
# legacy targets — the kernels the `.ko` backend can NOT serve (no GKI, no DDK):
#
#   4.4   (AOSP common)     — oldest requested Android family (offset-identical
#                             to 4.9; separate boot proves the loader + build)
#   4.9   (AOSP common)     — rt6_info IPv6 model
#   4.14  (AOSP common)     — issue #33; rt6_info IPv6 model
#   4.19  (AOSP common)     — pre-nexthop, struct fib6_info
#   5.4   (AOSP common)     — issue #35 (HyperOS) target
#
# Unlike build-kernel.sh (GKI built with the per-KMI DDK clang container), all
# five legacy kernels build with ONE pinned Bootlin aarch64 gcc — the compiler
# version doesn't affect struct layout (that's source+config), it's just old
# enough to compile these kernels (a modern gcc/clang trips on -Werror). The
# config is the branch's Android reference config (`cuttlefish_defconfig` on
# 4.x, `gki_defconfig` on 5.4) + qemu.config (virtio/console/initrd + software
# PAN) + the IPv6/policy-routing bits the harness's route/rule vectors need,
# matching the offsets in ../kpm/kver_offsets.h.
#
# Output is just an `Image` — the KPM never builds against the kernel tree
# (the `.kpm` is relocatable against KernelPatch headers), so unlike the `.ko`
# harness there is no tree/symvers to keep.
#
# Usage:  build-source-kernel.sh <4.4|4.9|4.14|4.19|5.4> [outdir]
# Output: <outdir>/Image            (default: .cache/legacy/<ver>/Image)
#
# Boot these with `-cpu cortex-a57` (they fault on `-cpu max`'s newer features
# before the console) and `rodata=off` — run-kpm.sh already does both.
set -euo pipefail

VER="${1:?usage: build-source-kernel.sh <4.4|4.9|4.14|4.19|5.4> [outdir]}"
HERE="$(cd "$(dirname "$0")" && pwd)"
CACHE="$HERE/.cache/legacy"
OUT="${2:-$CACHE/$VER}"
SRCROOT="$CACHE/src"
TCROOT="$CACHE/toolchain"
FRAG="$HERE/qemu.config"
JOBS="$(nproc)"
mkdir -p "$OUT" "$SRCROOT" "$TCROOT"

# --- one Bootlin aarch64 gcc 7.3 for every legacy kernel ----------------------
# Pinned release; the absolute CROSS_COMPILE prefix is used directly (NOT put on
# PATH — the toolchain ships a `bison`/`m4` that expects /opt and shadows the
# host kconfig tools otherwise).
TC="aarch64--glibc--stable-2018.11-1"
TC_URL="https://toolchains.bootlin.com/downloads/releases/toolchains/aarch64/tarballs/${TC}.tar.bz2"
CROSS="$TCROOT/$TC/bin/aarch64-buildroot-linux-gnu-"
if [ ! -x "${CROSS}gcc" ]; then
	echo "[legacy] fetching Bootlin gcc 7.3 ($TC)…"
	curl -fsSL "$TC_URL" | tar xj -C "$TCROOT"
fi

# --- pinned AOSP source -------------------------------------------------------
# Most `deprecated/android-*-q` refs are retained by the GitHub AOSP mirror;
# android11-5.4 and android-4.4-p come from the canonical Android Git server (the
# mirror can't shallow-fetch 4.4-p). Fetch the named branch shallowly, then
# require its tip to equal the pinned commit — branch movement cannot silently
# change the reference source.
AOSP_MIRROR="https://github.com/aosp-mirror/kernel_common.git"
AOSP_CANONICAL="https://android.googlesource.com/kernel/common"
checkout_aosp() {
	local remote="$1" branch="$2" sha="$3" dest="$4"

	if ! git -C "$dest" rev-parse --git-dir >/dev/null 2>&1; then
		echo "[legacy] fetching AOSP common $branch @ $sha…"
		# A leftover half-initialised dir (git init ran, fetch didn't finish)
		# would make the rev-parse above pass and the fetch below be skipped,
		# so start clean. googlesource intermittently short-packs a shallow
		# fetch of a deprecated branch ("remote did not send all necessary
		# objects"); it is not deterministic, so retry a few times before
		# giving up rather than failing the whole legacy-image bake on a flake.
		rm -rf "$dest"
		mkdir -p "$dest"
		git -C "$dest" init -q
		git -C "$dest" remote add origin "$remote"
		local attempt
		for attempt in 1 2 3 4 5; do
			if git -C "$dest" fetch --depth=1 origin "refs/heads/$branch"; then
				break
			fi
			echo "[legacy] fetch attempt $attempt failed; retrying…" >&2
			# Drop whatever partial pack landed so the retry starts fresh.
			rm -rf "$dest/.git/shallow" "$dest"/.git/objects/pack/tmp_* 2>/dev/null || true
			sleep 3
			[ "$attempt" = 5 ] && { echo "ERROR: could not fetch $branch after 5 tries" >&2; exit 2; }
		done
		git -C "$dest" checkout -q --detach FETCH_HEAD
	fi
	if [ "$(git -C "$dest" rev-parse HEAD)" != "$sha" ]; then
		echo "ERROR: $dest is not pinned at $sha" >&2
		exit 2
	fi
}

case "$VER" in
4.4)
	SRC="$SRCROOT/aosp-4.4"
	# The GitHub AOSP mirror fails a shallow fetch of this deprecated branch
	# ("remote did not send all necessary objects"), so 4.4 uses the canonical
	# Android Git server, which serves it cleanly.
	checkout_aosp "$AOSP_CANONICAL" deprecated/android-4.4-p \
		875c0cc8115381f702b12d41de293807f47cdac9 "$SRC"
	DEFCONFIG=cuttlefish_defconfig
	;;
4.9)
	SRC="$SRCROOT/aosp-4.9"
	checkout_aosp "$AOSP_MIRROR" deprecated/android-4.9-q \
		f9b8314c64640cd10c7b14ce9d2a11a0dc02a941 "$SRC"
	DEFCONFIG=cuttlefish_defconfig
	;;
4.14)
	SRC="$SRCROOT/aosp-4.14"
	checkout_aosp "$AOSP_MIRROR" deprecated/android-4.14-q \
		ef7460eabd6dc0cf87ee5ead6fd0d0b6c8c288f2 "$SRC"
	DEFCONFIG=cuttlefish_defconfig
	;;
4.19)
	SRC="$SRCROOT/aosp-4.19"
	checkout_aosp "$AOSP_MIRROR" deprecated/android-4.19-q \
		12d190ed22fbc3ac4b9b19b9305f26d530769fc8 "$SRC"
	DEFCONFIG=cuttlefish_defconfig
	;;
5.4)
	SRC="$SRCROOT/aosp-5.4"
	checkout_aosp "$AOSP_CANONICAL" android11-5.4 \
		91d385eb2a413918a037a93089f8e29910fdf707 "$SRC"
	DEFCONFIG=gki_defconfig
	;;
*)
	echo "ERROR: unknown version '$VER' (expected 4.4 | 4.9 | 4.14 | 4.19 | 5.4)"; exit 2 ;;
esac

cd "$SRC"
mk() { make ARCH=arm64 CROSS_COMPILE="$CROSS" "$@"; }

# --- config: defconfig + qemu.config + IPv6/policy, per-version tweaks ---------
mk "$DEFCONFIG" >/dev/null
./scripts/kconfig/merge_config.sh -m .config "$FRAG" >/dev/null 2>&1

# IPv6 + policy routing built-in so the v6-route / policy-rule vectors have
# something to hide (the generic defconfig ships IPv6 as a module / off).
scripts/config --enable IPV6 \
	--enable IP_ADVANCED_ROUTER --enable IP_MULTIPLE_TABLES \
	--enable IPV6_MULTIPLE_TABLES --enable IPV6_SUBTREES \
	--disable MODULE_SIG --disable TRIM_UNUSED_KSYMS --disable DEBUG_INFO_BTF

# 4.19 only: fib6_info has a CONFIG_IPV6_ROUTER_PREF-gated `last_probe` field,
# and kver_offsets.h pins fib6_nh@176 (the ROUTER_PREF=y, Android-common shape).
[ "$VER" = "4.19" ] && scripts/config --enable IPV6_ROUTER_PREF --enable IPV6_ROUTE_INFO

mk olddefconfig >/dev/null
echo "[legacy] $VER config: $(grep -hE '^CONFIG_(NF_CONNTRACK[=_]|IPV6=|IP_MULTIPLE_TABLES=|IPV6_ROUTER_PREF=|ARM64_SW_TTBR0_PAN=)' .config | tr '\n' ' ')"

# --- build ---
echo "[legacy] $VER: building Image (gcc, -j$JOBS)…"
mk -j"$JOBS" Image
cp arch/arm64/boot/Image "$OUT/Image"
echo "[legacy] $VER done -> $OUT/Image"
