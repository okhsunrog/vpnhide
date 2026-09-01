#!/usr/bin/env bash
# Build a QEMU-bootable GKI kernel with the vpnhide built-in backend baked in
# for <kmi>, into the per-KMI cache so run.sh can boot it. Unlike
# kmod/test/build-kernel.sh there is NO module: builtin/scripts/apply.sh is
# applied to the source tree and CONFIG_VPNHIDE=y is merged, so the driver ends
# up compiled into the Image.
#
# Usage:  builtin/test/build-kernel.sh <kmi>          e.g. android14-6.1
# Output: builtin/test/.cache/<kmi>/Image
set -euo pipefail

KMI="${1:?usage: build-kernel.sh <kmi>  (e.g. android14-6.1)}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
CACHE="$HERE/.cache/$KMI"
FRAG="$REPO/kmod/test/qemu.config"   # reuse the proven QEMU boot fragment

DDK_IMAGE_TAG="20260313"
DDK="${VPNHIDE_DDK_IMAGE:-ghcr.io/ylarod/ddk-min:${KMI}-${DDK_IMAGE_TAG}}"

CONTAINER_CMD="${VPNHIDE_CONTAINER_RUNTIME:-}"
if [ -z "$CONTAINER_CMD" ]; then
	if command -v podman >/dev/null 2>&1; then CONTAINER_CMD="podman"
	elif command -v docker >/dev/null 2>&1; then CONTAINER_CMD="docker"
	else echo "ERROR: neither podman nor docker found"; exit 1; fi
fi

mkdir -p "$CACHE"
echo "[build-kernel/builtin] $KMI: cloning kernel/common + baking CONFIG_VPNHIDE + building Image (slow)…"

"$CONTAINER_CMD" run --rm \
	-v "$REPO:/repo:ro" -v "$CACHE:/out" -v "$FRAG:/qemu.config:ro" \
	-e KMI="$KMI" "$DDK" bash -euo pipefail -c '
	CLANG_BIN="$(ls -d /opt/ddk/clang/*/bin | head -1)"
	export PATH="$CLANG_BIN:$PATH"

	git clone --depth=1 -b "$KMI" \
		https://android.googlesource.com/kernel/common /tmp/linux

	# Bake the built-in backend into the source tree.
	/repo/builtin/scripts/apply.sh /tmp/linux "$KMI"

	cd /tmp/linux
	make ARCH=arm64 LLVM=1 gki_defconfig
	cp /qemu.config /tmp/frag.config
	printf "CONFIG_VPNHIDE=y\nCONFIG_VPNHIDE_FS_HIDING=y\n" >> /tmp/frag.config
	./scripts/kconfig/merge_config.sh -m .config /tmp/frag.config
	make ARCH=arm64 LLVM=1 olddefconfig
	grep -E "CONFIG_VPNHIDE(_FS_HIDING)?=" .config
	make ARCH=arm64 LLVM=1 -j"$(nproc)" Image
	cp arch/arm64/boot/Image /out/Image
'

echo "[build-kernel/builtin] $KMI: done"
echo "  Image: $CACHE/Image"
