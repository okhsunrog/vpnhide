#!/usr/bin/env bash
# Prove that kmod/test/qemu-trim.config changes no structure layout.
#
# The trim fragment switches off subsystems the QEMU harness never reaches. A
# Kconfig symbol can also move a member of a structure the backends read, and
# the KPM backend reads those offsets from a hardcoded table
# (kmod/kpm/kver_offsets.h) rather than from headers — so a trim that shifted an
# offset would make kpm-qemu fail, or worse, pass against a layout no device
# has. This script is the guard: it configures the same kernel tree twice, with
# and without the trim, compiles kmod/test/abi/abi-probe.c against each, and
# diffs the pahole layout of every structure the backends dereference. Any
# difference is a failure.
#
# It compiles one object per variant, not a kernel — layouts come from the
# headers, so `make modules_prepare` plus a single .o is enough, and the whole
# run is minutes rather than two full builds.
#
# Usage:
#   kmod/test/verify-trim-abi.sh <kmi> [--commit <sha>]
#       spawn the matching DDK container and clone kernel/common (developer run)
#   kmod/test/verify-trim-abi.sh <kmi> --inside-container [--ksrc <dir>]
#       run in the current container against an existing, CLEAN source tree
#       (the image bake; a tree that was already built in-tree will be rejected
#       by kbuild's O= check, so run this before the in-tree build)
#
# Output: the two layout dumps and their diff under --work (default: a temp dir).
set -euo pipefail

KMI=""
COMMIT=""
KSRC=""
WORK=""
INSIDE=0

while [ $# -gt 0 ]; do
	case "$1" in
	--inside-container) INSIDE=1 ;;
	--commit) COMMIT="$2"; shift ;;
	--ksrc) KSRC="$2"; shift ;;
	--work) WORK="$2"; shift ;;
	-h | --help)
		sed -n '2,25p' "$0"
		exit 0
		;;
	-*)
		echo "unknown option: $1" >&2
		exit 2
		;;
	*) KMI="$1" ;;
	esac
	shift
done

: "${KMI:?usage: verify-trim-abi.sh <kmi> [--inside-container] [--ksrc DIR] [--commit SHA]}"

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

# ---------------------------------------------------------------- outer half
# Not in the DDK container yet: re-run this same script inside it. Mirrors
# kmod/test/build-kernel.sh (same image tag, same runtime detection).
if [ "$INSIDE" -eq 0 ]; then
	DDK_IMAGE_TAG="20260313"
	DDK="${VPNHIDE_DDK_IMAGE:-ghcr.io/ylarod/ddk-min:${KMI}-${DDK_IMAGE_TAG}}"

	CONTAINER_CMD="${VPNHIDE_CONTAINER_RUNTIME:-}"
	if [ -z "$CONTAINER_CMD" ]; then
		if command -v podman >/dev/null 2>&1; then CONTAINER_CMD="podman"
		elif command -v docker >/dev/null 2>&1; then CONTAINER_CMD="docker"
		else echo "ERROR: neither podman nor docker found" >&2; exit 1; fi
	fi

	HOST_WORK="${WORK:-$HERE/.cache/abi-$KMI}"
	mkdir -p "$HOST_WORK"
	echo "[verify-trim-abi] $KMI: work dir $HOST_WORK"

	exec "$CONTAINER_CMD" run --rm \
		-v "$REPO:/repo:ro" -v "$HOST_WORK:/work" \
		"$DDK" /repo/kmod/test/verify-trim-abi.sh "$KMI" \
		--inside-container --work /work ${COMMIT:+--commit "$COMMIT"}
fi

# ---------------------------------------------------------------- inner half
WORK="${WORK:-$(mktemp -d)}"
mkdir -p "$WORK"

command -v pahole >/dev/null 2>&1 || {
	echo "ERROR: pahole not found in this image" >&2
	exit 1
}

CLANG_BIN="$(printf '%s\n' /opt/ddk/clang/*/bin | head -1)"
export PATH="$CLANG_BIN:$PATH"

if [ -z "$KSRC" ]; then
	KSRC="$WORK/linux"
	if [ ! -d "$KSRC/.git" ]; then
		echo "[verify-trim-abi] cloning kernel/common ($KMI${COMMIT:+ @ $COMMIT})…"
		if [ -n "$COMMIT" ]; then
			git init "$KSRC"
			git -C "$KSRC" remote add origin \
				https://android.googlesource.com/kernel/common
			git -C "$KSRC" fetch --depth=1 origin "$COMMIT"
			git -C "$KSRC" checkout --detach FETCH_HEAD
		else
			git clone --depth=1 -b "$KMI" \
				https://android.googlesource.com/kernel/common "$KSRC"
		fi
	fi
fi

echo "[verify-trim-abi] kernel tree: $KSRC ($(git -C "$KSRC" rev-parse --short HEAD 2>/dev/null || echo 'not a git tree'))"

# The probe is built out-of-tree; /repo may be read-only, so copy it.
ABI="$WORK/abi"
rm -rf "$ABI"
cp -r "$REPO/kmod/test/abi" "$ABI"

# The structure list is the probe itself — one source of truth.
mapfile -t STRUCTS < <(sed -n 's/^VPNHIDE_ABI_PROBE(\([a-z_0-9]*\));.*/\1/p' "$ABI/abi-probe.c")
[ "${#STRUCTS[@]}" -gt 0 ] || {
	echo "ERROR: no VPNHIDE_ABI_PROBE() entries found in abi-probe.c" >&2
	exit 1
}
echo "[verify-trim-abi] ${#STRUCTS[@]} structures to compare"

# pahole needs real DWARF, and the trimmed variant is configured with
# CONFIG_DEBUG_INFO_NONE. Forcing -g through KCFLAGS is not enough: on 6.12
# AutoFDO appends -gmlt *after* KCFLAGS whenever CONFIG_DEBUG_INFO is unset,
# which leaves line tables and no types at all. So debug info is turned back on
# through the config — for BOTH variants, after the real one has been checked,
# so the comparison stays about the trim and nothing else. Debug info is
# metadata; it cannot move a member.
DEBUG_FRAG="$WORK/abi-debuginfo.config"
cat >"$DEBUG_FRAG" <<'EOF'
CONFIG_DEBUG_INFO=y
CONFIG_DEBUG_INFO_DWARF4=y
EOF

dump_variant() {
	local name="$1" # base | trim
	local build="$WORK/abi-$name"
	local out="$WORK/layout-$name.txt"
	local frags=("$REPO/kmod/test/qemu.config")
	[ "$name" = "trim" ] && frags+=("$REPO/kmod/test/qemu-trim.config")

	echo "[verify-trim-abi] === $name: configure + build the probe ==="
	rm -rf "$build"
	mkdir -p "$build"
	# Pass 1: the config as it will actually be built in CI — this is what the
	# checks below inspect.
	(
		cd "$KSRC"
		make ARCH=arm64 LLVM=1 O="$build" gki_defconfig >/dev/null
		./scripts/kconfig/merge_config.sh -O "$build" -m "$build/.config" "${frags[@]}" >/dev/null
		make ARCH=arm64 LLVM=1 O="$build" olddefconfig >"$build/olddefconfig.log" 2>&1
	)
	cp "$build/.config" "$WORK/config-$name"

	# Switching a subsystem off while a GKI_HIDDEN_* dummy still `select`s its
	# helpers leaves kconfig in a state no shipped kernel is configured in.
	# Treat it as a failure rather than let it scroll past in a build log.
	if grep -q "unmet direct dependencies" "$build/olddefconfig.log"; then
		echo "ERROR: $name config has unmet dependencies:" >&2
		grep -A3 "unmet direct dependencies" "$build/olddefconfig.log" >&2
		exit 1
	fi

	# A fragment line can be silently overridden: GKI's dummy GKI_HIDDEN_*
	# options `select` helper symbols, so `# CONFIG_MEDIA_SUPPORT is not set`
	# had no effect at all until GKI_HIDDEN_MEDIA_CONFIGS went with it. Check
	# every request landed. A tristate that fell back to =m is accepted —
	# `make Image` builds no obj-m, which is the point.
	if [ "$name" = "trim" ]; then
		local sym ineffective=0
		while read -r sym; do
			if grep -q "^${sym}=y$" "$build/.config"; then
				echo "  ineffective: $sym is still built in" >&2
				ineffective=1
			fi
		done < <(sed -n 's/^# \(CONFIG_[A-Z0-9_]*\) is not set$/\1/p' \
			"$REPO/kmod/test/qemu-trim.config")
		local name_only
		while read -r sym; do
			grep -q "^${sym}$" "$build/.config" && continue
			# A symbol this kernel does not have at all (older KMIs lack
			# DEBUG_INFO_NONE, for one) is not a failure — the fragment covers
			# those with a second spelling. A symbol that IS there with another
			# value was overridden, and that is.
			name_only="${sym%%=*}"
			if grep -q "^${name_only}=\|^# ${name_only} is not set$" "$build/.config"; then
				echo "  ineffective: $sym was overridden" >&2
				ineffective=1
			else
				echo "  n/a on this KMI: $sym"
			fi
		done < <(grep -E '^CONFIG_[A-Z0-9_]+=y$' "$REPO/kmod/test/qemu-trim.config")
		[ "$ineffective" -eq 0 ] || {
			echo "ERROR: qemu-trim.config has lines the kernel ignored" >&2
			exit 1
		}
	fi

	# Pass 2: the same config plus debug info, which is what the probe is
	# compiled against. Only the DWARF differs.
	(
		cd "$KSRC"
		./scripts/kconfig/merge_config.sh -O "$build" -m "$build/.config" "$DEBUG_FRAG" >/dev/null
		make ARCH=arm64 LLVM=1 O="$build" olddefconfig >/dev/null 2>&1
		make ARCH=arm64 LLVM=1 O="$build" -j"$(nproc)" modules_prepare >/dev/null
		# -fno-lto: the full-LTO KMIs (5.10, 5.15) would otherwise emit LLVM
		# bitcode, which carries no DWARF for pahole to read. -fno-sanitize=cfi
		# has to go with it, because on those KMIs CFI is implemented through
		# LTO and clang rejects one without the other. Both change how this one
		# object is generated, never a structure's layout, and the kernel itself
		# is still built with whatever its KMI prescribes.
		make ARCH=arm64 LLVM=1 O="$build" M="$ABI" \
			KCFLAGS="-fno-lto -fno-sanitize=cfi" \
			-j"$(nproc)" abi-probe.o >/dev/null
	)
	grep -q '^CONFIG_DEBUG_INFO=y$' "$build/.config" || {
		echo "ERROR: could not turn debug info back on for the $name probe" >&2
		exit 1
	}

	: >"$out"
	local s layout
	for s in "${STRUCTS[@]}"; do
		# One pahole call, captured whole: piping it into anything that exits
		# early (`grep -q`) kills pahole with SIGPIPE, and under `pipefail` a
		# big structure then looks like a missing one.
		layout="$(pahole -C "$s" "$ABI/abi-probe.o" 2>/dev/null || true)"
		{
			echo "===== struct $s ====="
			if [ -z "$layout" ]; then
				echo "MISSING: pahole found no definition"
			else
				printf '%s\n' "$layout"
			fi
		} >>"$out"
	done
	rm -f "$ABI/abi-probe.o"
}

dump_variant base
dump_variant trim

# A type the probe cannot emit would compare equal to itself and quietly leave
# that structure unguarded — a renamed or misspelled entry must fail loudly.
if grep -q MISSING "$WORK/layout-base.txt"; then
	echo "ERROR: no DWARF for these structures — fix kmod/test/abi/abi-probe.c:" >&2
	grep -B1 MISSING "$WORK/layout-base.txt" | grep '=====' >&2
	exit 1
fi

echo
if diff -u "$WORK/layout-base.txt" "$WORK/layout-trim.txt" >"$WORK/layout.diff"; then
	echo "[verify-trim-abi] PASS — ${#STRUCTS[@]} structures identical with and without the trim"
	echo "  layouts: $WORK/layout-base.txt, $WORK/layout-trim.txt"
	# A config that differs in nothing would make the PASS meaningless.
	if diff -q "$WORK/config-base" "$WORK/config-trim" >/dev/null; then
		echo "ERROR: the two .config files are identical — the fragment did nothing" >&2
		exit 1
	fi
	echo "  .config deltas: $(diff "$WORK/config-base" "$WORK/config-trim" | grep -c '^[<>]') lines"
	exit 0
fi

echo "[verify-trim-abi] FAIL — the trim moved a structure the backends read:" >&2
sed -n '1,80p' "$WORK/layout.diff" >&2
echo "  full diff: $WORK/layout.diff" >&2
exit 1
