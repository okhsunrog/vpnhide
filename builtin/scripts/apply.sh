#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# =============================================================================
# apply.sh — integrate the vpnhide in-tree backend into a GKI kernel source tree
#
#   apply.sh <kernel_common_dir> <version>
#
#     version: android14-6.1   (more added as their patch sets land)
#
# What it does (idempotent):
#   1. copy the driver           -> <kernel>/security/vpnhide/
#   2. vendor the shared logic    (kmod/shared/vpnhide_logic.h) and generated
#      tables (kmod/generated/{iface_lists,hook_ids}.h) into the driver dir, so
#      the repo keeps ONE source of truth (regenerate with scripts/codegen-*.py)
#   3. copy the public header     -> <kernel>/include/linux/vpnhide.h
#   4. wire security/Kconfig + security/Makefile to build it
#   5. apply the per-version call-site patches under builtin/versions/<version>/
#
# Then build the kernel normally with CONFIG_VPNHIDE=y (and, optionally,
# CONFIG_VPNHIDE_FS_HIDING=y) via the tree's usual GKI/Kleaf workflow.
# =============================================================================
set -euo pipefail

KERNEL_DIR="${1:?Usage: $0 <kernel_common_dir> <version>}"
VERSION="${2:?Usage: $0 <kernel_common_dir> <version>}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILTIN_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$BUILTIN_DIR/.." && pwd)"

DRIVER_SRC="$BUILTIN_DIR/security/vpnhide"
HEADER_SRC="$BUILTIN_DIR/include/linux/vpnhide.h"
PATCHES_DIR="$BUILTIN_DIR/versions/$VERSION"
SHARED_SRC="$REPO_ROOT/kmod/shared/vpnhide_logic.h"
GENERATED_SRC="$REPO_ROOT/kmod/generated"

log() { echo "[apply.sh] $*"; }
die() { echo "[apply.sh] ERROR: $*" >&2; exit 1; }

[ -d "$KERNEL_DIR" ]   || die "kernel dir not found: $KERNEL_DIR"
[ -d "$DRIVER_SRC" ]   || die "driver source not found: $DRIVER_SRC"
[ -f "$HEADER_SRC" ]   || die "public header not found: $HEADER_SRC"
[ -f "$SHARED_SRC" ]   || die "shared logic not found: $SHARED_SRC (run scripts/codegen-*.py?)"
[ -d "$PATCHES_DIR" ]  || die "no patches for version '$VERSION' (dir missing: $PATCHES_DIR)"
[ -f "$KERNEL_DIR/security/Kconfig" ]  || die "not a kernel tree: no security/Kconfig"

# --------------------------------------------------------------------------
# 1. driver
# --------------------------------------------------------------------------
log "Copying security/vpnhide driver..."
rm -rf "$KERNEL_DIR/security/vpnhide"
mkdir -p "$KERNEL_DIR/security/vpnhide/shared" \
	 "$KERNEL_DIR/security/vpnhide/generated"
cp "$DRIVER_SRC"/*.c "$DRIVER_SRC"/*.h "$DRIVER_SRC/Kconfig" \
	"$DRIVER_SRC/Makefile" "$KERNEL_DIR/security/vpnhide/"

# --------------------------------------------------------------------------
# 2. vendor shared logic + generated tables (single source of truth: kmod/)
# --------------------------------------------------------------------------
log "Vendoring shared logic + generated tables from kmod/..."
cp "$SHARED_SRC" "$KERNEL_DIR/security/vpnhide/shared/vpnhide_logic.h"
cp "$GENERATED_SRC/iface_lists.h" "$GENERATED_SRC/hook_ids.h" \
	"$KERNEL_DIR/security/vpnhide/generated/"

# --------------------------------------------------------------------------
# 3. public header
# --------------------------------------------------------------------------
log "Copying include/linux/vpnhide.h..."
cp "$HEADER_SRC" "$KERNEL_DIR/include/linux/vpnhide.h"

# --------------------------------------------------------------------------
# 4. wire security/Kconfig + security/Makefile (idempotent, version-stable)
# --------------------------------------------------------------------------
KCONFIG="$KERNEL_DIR/security/Kconfig"
if ! grep -q 'security/vpnhide/Kconfig' "$KCONFIG"; then
	log "Wiring security/Kconfig..."
	# Insert the source line just before the closing 'endmenu'.
	awk '
		/^endmenu/ && !done {
			print "source \"security/vpnhide/Kconfig\""
			done = 1
		}
		{ print }
	' "$KCONFIG" > "$KCONFIG.tmp" && mv "$KCONFIG.tmp" "$KCONFIG"
	grep -q 'security/vpnhide/Kconfig' "$KCONFIG" \
		|| die "failed to wire security/Kconfig (no endmenu?)"
else
	log "security/Kconfig already wired, skipping."
fi

SEC_MAKEFILE="$KERNEL_DIR/security/Makefile"
if ! grep -q 'CONFIG_VPNHIDE' "$SEC_MAKEFILE"; then
	log "Wiring security/Makefile..."
	printf '\nobj-$(CONFIG_VPNHIDE)\t\t+= vpnhide/\n' >> "$SEC_MAKEFILE"
else
	log "security/Makefile already wired, skipping."
fi

# --------------------------------------------------------------------------
# 5. per-version call-site patches
# --------------------------------------------------------------------------
shopt -s nullglob
PATCHES=("$PATCHES_DIR"/*.patch)
shopt -u nullglob
[ "${#PATCHES[@]}" -gt 0 ] || die "no .patch files in $PATCHES_DIR"

for p in $(printf '%s\n' "${PATCHES[@]}" | sort); do
	log "Applying $(basename "$p")..."
	patch -p1 --forward --fuzz=3 --no-backup-if-mismatch \
		-d "$KERNEL_DIR" < "$p" \
		|| die "patch failed: $p"
done

log "Done. Enable CONFIG_VPNHIDE=y (and CONFIG_VPNHIDE_FS_HIDING=y for path"
log "concealment) in your defconfig, then build the kernel as usual."
