#!/usr/bin/env bash
# Build a QEMU-bootable GKI kernel + the vpnhide module for <kmi> into the
# per-KMI cache, so run.sh can boot them. This is the expensive, canonical
# producer: it clones kernel/common at the matching branch, merges qemu.config
# on top of gki_defconfig, builds Image with the DDK container's matching
# clang, then builds the module against that exact tree (so it loads cleanly).
#
# Stock GKI boot images can't boot in `qemu -machine virt` (no virtio/console
# drivers); this is how the bootable test kernel is produced. Output is cached
# and reused — only re-run when the kernel branch or qemu.config changes.
#
# Usage:  kmod/test/build-kernel.sh <kmi>          e.g. android14-6.1
# Output: kmod/test/.cache/<kmi>/{Image,vpnhide_kmod.ko}
set -euo pipefail

KMI="${1:?usage: build-kernel.sh <kmi>  (e.g. android14-6.1)}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
CACHE="$HERE/.cache/$KMI"
FRAG="$HERE/qemu.config"

# Mirror DDK_IMAGE_TAG in kmod/build.py so the kernel + module use the same
# toolchain as the module CI build.
DDK_IMAGE_TAG="20260313"
DDK="ghcr.io/ylarod/ddk-min:${KMI}-${DDK_IMAGE_TAG}"

mkdir -p "$CACHE"
echo "[build-kernel] $KMI: cloning kernel/common + building Image (this is slow)…"

docker run --rm \
	-v "$REPO:/repo:ro" -v "$CACHE:/out" -v "$FRAG:/qemu.config:ro" \
	-e KMI="$KMI" "$DDK" bash -euo pipefail -c '
	CLANG_BIN="$(ls -d /opt/ddk/clang/*/bin | head -1)"
	export PATH="$CLANG_BIN:$PATH"

	# kernel/common branch name == the KMI label
	git clone --depth=1 -b "$KMI" \
		https://android.googlesource.com/kernel/common /tmp/linux
	cd /tmp/linux
	# Work around an AOSP host-tool regression on newer GKI HEADs: certs/
	# extract-cert.c declares key_pass only under USE_PKCS11_ENGINE but uses it
	# in the non-BoringSSL ENGINE branch, breaking the host build. Unrelated to
	# vpnhide; make the declaration unconditional. Idempotent.
	sed -i -z "s/#ifdef USE_PKCS11_ENGINE\nstatic const char \*key_pass;\n#endif/static const char *key_pass;/" \
		certs/extract-cert.c || true
	make ARCH=arm64 LLVM=1 gki_defconfig
	./scripts/kconfig/merge_config.sh -m .config /qemu.config
	make ARCH=arm64 LLVM=1 olddefconfig
	make ARCH=arm64 LLVM=1 -j"$(nproc)" Image
	make ARCH=arm64 LLVM=1 modules_prepare
	cp arch/arm64/boot/Image /out/Image
	# external-module builds need Module.symvers (symbol CRCs); make Image only
	# emits vmlinux.symvers — our module references only vmlinux symbols, so that
	# is sufficient. Without it modpost reports every kernel symbol undefined.
	cp vmlinux.symvers Module.symvers

	# build the module against THIS tree (matching CFI / vermagic / struct
	# layouts), not the GKI kdir — building against the kdir mismatches the
	# separately-built kernel and panics on some KMIs (CFI / kretprobe on 6.12).
	cp -r /repo/kmod /tmp/kmod-build
	make -C /tmp/kmod-build KERNEL_SRC=/tmp/linux CLANG_DIR="$CLANG_BIN" clean || true
	make -C /tmp/kmod-build KERNEL_SRC=/tmp/linux CLANG_DIR="$CLANG_BIN"
	cp /tmp/kmod-build/vpnhide_kmod.ko /out/vpnhide_kmod.ko
'

echo "[build-kernel] $KMI: done"
echo "  Image:  $CACHE/Image"
echo "  module: $CACHE/vpnhide_kmod.ko"
