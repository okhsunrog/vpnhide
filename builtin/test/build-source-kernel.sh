#!/usr/bin/env bash
# Build a QEMU-bootable, from-source LEGACY kernel with the vpnhide built-in
# backend baked in — the pre-GKI families the per-KMI DDK containers don't cover
# (there is no ddk-min image below android12-5.10). Same pinned AOSP sources and
# Bootlin gcc 7.3 as kmod/test/build-source-kernel.sh (that compiler is old
# enough to build these trees; a modern clang trips on -Werror), but instead of
# leaving the tree stock this applies builtin/scripts/apply.sh + CONFIG_VPNHIDE=y
# so the driver is compiled into the Image.
#
#   4.9 / 4.14 / 4.19  (AOSP common, cuttlefish_defconfig)
#   5.4                (AOSP common, gki_defconfig)
#
# Usage:  builtin/test/build-source-kernel.sh <4.9|4.14|4.19|5.4> [outdir]
# Output: <outdir>/Image   (default: builtin/test/.cache/legacy/<ver>/Image)
#
# Boot with `-cpu cortex-a57` + `rodata=off` (these fault on `-cpu max`) —
# builtin/test/run.sh does both when VPNHIDE_QEMU_CPU=cortex-a57 is set.
set -euo pipefail

VER="${1:?usage: build-source-kernel.sh <4.9|4.14|4.19|5.4> [outdir]}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
CACHE="$HERE/.cache/legacy"
OUT="${2:-$CACHE/$VER}"
SRCROOT="$CACHE/src"
# Reuse the kmod harness's toolchain download if present, else fetch our own.
TCROOT="${VPNHIDE_LEGACY_TOOLCHAIN_DIR:-$REPO/kmod/test/.cache/legacy/toolchain}"
FRAG="$REPO/kmod/test/qemu.config"
JOBS="$(nproc)"
mkdir -p "$OUT" "$SRCROOT" "$TCROOT"

# --- one Bootlin aarch64 gcc 7.3 for every legacy kernel ----------------------
TC="aarch64--glibc--stable-2018.11-1"
TC_URL="https://toolchains.bootlin.com/downloads/releases/toolchains/aarch64/tarballs/${TC}.tar.bz2"
CROSS="$TCROOT/$TC/bin/aarch64-buildroot-linux-gnu-"
if [ ! -x "${CROSS}gcc" ]; then
	echo "[legacy] fetching Bootlin gcc 7.3 ($TC)…"
	curl -fsSL "$TC_URL" | tar xj -C "$TCROOT"
fi

# --- pinned AOSP source (same refs/SHAs as kmod/test/build-source-kernel.sh) ---
AOSP_MIRROR="https://github.com/aosp-mirror/kernel_common.git"
AOSP_CANONICAL="https://android.googlesource.com/kernel/common"
checkout_aosp() {
	local remote="$1" branch="$2" sha="$3" dest="$4"
	if ! git -C "$dest" rev-parse --git-dir >/dev/null 2>&1; then
		echo "[legacy] fetching AOSP common $branch @ $sha…"
		mkdir -p "$dest"
		git -C "$dest" init -q
		git -C "$dest" remote add origin "$remote"
		git -C "$dest" fetch --depth=1 origin "refs/heads/$branch"
		git -C "$dest" checkout -q --detach FETCH_HEAD
	fi
	if [ "$(git -C "$dest" rev-parse HEAD)" != "$sha" ]; then
		echo "ERROR: $dest is not pinned at $sha" >&2
		exit 2
	fi
}

case "$VER" in
4.9)
	SRC="$SRCROOT/aosp-4.9"; KMI=android10-4.9
	checkout_aosp "$AOSP_MIRROR" deprecated/android-4.9-q \
		f9b8314c64640cd10c7b14ce9d2a11a0dc02a941 "$SRC"
	DEFCONFIG=cuttlefish_defconfig ;;
4.14)
	SRC="$SRCROOT/aosp-4.14"; KMI=android10-4.14
	checkout_aosp "$AOSP_MIRROR" deprecated/android-4.14-q \
		ef7460eabd6dc0cf87ee5ead6fd0d0b6c8c288f2 "$SRC"
	DEFCONFIG=cuttlefish_defconfig ;;
4.19)
	SRC="$SRCROOT/aosp-4.19"; KMI=android10-4.19
	checkout_aosp "$AOSP_MIRROR" deprecated/android-4.19-q \
		12d190ed22fbc3ac4b9b19b9305f26d530769fc8 "$SRC"
	DEFCONFIG=cuttlefish_defconfig ;;
5.4)
	SRC="$SRCROOT/aosp-5.4"; KMI=android11-5.4
	checkout_aosp "$AOSP_CANONICAL" android11-5.4 \
		91d385eb2a413918a037a93089f8e29910fdf707 "$SRC"
	DEFCONFIG=gki_defconfig ;;
*)
	echo "ERROR: unknown version '$VER' (expected 4.9 | 4.14 | 4.19 | 5.4)"; exit 2 ;;
esac

# --- bake the built-in backend into the source tree ---------------------------
# apply.sh is idempotent; a stray build tree is reused across reruns.
"$REPO/builtin/scripts/apply.sh" "$SRC" "$KMI"

cd "$SRC"
mk() { make ARCH=arm64 CROSS_COMPILE="$CROSS" "$@"; }

# --- config: defconfig + qemu.config + IPv6/policy + CONFIG_VPNHIDE ------------
mk "$DEFCONFIG" >/dev/null
./scripts/kconfig/merge_config.sh -m .config "$FRAG" >/dev/null 2>&1
scripts/config --enable IPV6 \
	--enable IP_ADVANCED_ROUTER --enable IP_MULTIPLE_TABLES \
	--enable IPV6_MULTIPLE_TABLES --enable IPV6_SUBTREES \
	--disable MODULE_SIG --disable TRIM_UNUSED_KSYMS --disable DEBUG_INFO_BTF \
	--enable VPNHIDE --enable VPNHIDE_FS_HIDING
[ "$VER" = "4.19" ] && scripts/config --enable IPV6_ROUTER_PREF --enable IPV6_ROUTE_INFO
mk olddefconfig >/dev/null
grep -E "CONFIG_VPNHIDE(_FS_HIDING)?=" .config || {
	echo "ERROR: CONFIG_VPNHIDE not enabled after olddefconfig" >&2; exit 2; }

echo "[legacy] $VER: building Image (gcc 7.3, -j$JOBS)…"
mk -j"$JOBS" Image
cp arch/arm64/boot/Image "$OUT/Image"
echo "[legacy] $VER done -> $OUT/Image"
