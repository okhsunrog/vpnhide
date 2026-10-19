# Gzipped boot/kernel partition images into $VPNHIDE_OUT_DIR (set by core, inside the
# app's data dir; core hands ownership to the app afterwards). Exit 0 only when at least
# one image was captured.
set +e
OUT_DIR="$VPNHIDE_OUT_DIR"
# Kernel/boot images are tens of MB; cap the copy so an unexpectedly large or
# mislabelled partition can't fill /data. 512 MiB = 128 * 4 MiB.
MAX_IMAGE_BYTES=536870912
MAX_IMAGE_BLOCKS=128
[ -n "$OUT_DIR" ] || exit 1
mkdir -p "$OUT_DIR" || exit 1
MANIFEST="$OUT_DIR/manifest.txt"
SLOT_SUFFIX="$(getprop ro.boot.slot_suffix 2>/dev/null)"
{
  echo "schema=1"
  echo "slot_suffix=$SLOT_SUFFIX"
  echo "created_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date)"
  echo "included=boot init_boot vendor_boot vendor_kernel_boot"
  echo "skipped=vendor_dlkm (metadata only; usually large and not a kernel image)"
  echo
} > "$MANIFEST"

resolve_partition() {
  NAME="$1"
  for CANDIDATE in "$NAME$SLOT_SUFFIX" "$NAME"; do
    PATH_TO_READ="/dev/block/by-name/$CANDIDATE"
    if [ -e "$PATH_TO_READ" ]; then
      echo "$PATH_TO_READ"
      return 0
    fi
  done
  return 1
}

HAS_IMAGE=0
for NAME in boot init_boot vendor_boot vendor_kernel_boot; do
  PATH_TO_READ="$(resolve_partition "$NAME" 2>/dev/null || true)"
  {
    echo "### $NAME"
    if [ -z "$PATH_TO_READ" ]; then
      echo "present=0"
      echo
      continue
    fi
    SIZE="$(blockdev --getsize64 "$PATH_TO_READ" 2>/dev/null || echo unknown)"
    OUT_BASE="$NAME$SLOT_SUFFIX.img"
    RAW="$OUT_DIR/$OUT_BASE"
    GZ="$OUT_DIR/$OUT_BASE.gz"
    echo "present=1"
    echo "path=$PATH_TO_READ"
    echo "size_bytes=$SIZE"
    case "$SIZE" in
      ''|*[!0-9]*)
        echo "skipped=size unknown (refusing unbounded dd)"
        echo
        continue
        ;;
    esac
    if [ "$SIZE" -gt "$MAX_IMAGE_BYTES" ]; then
      echo "skipped=size $SIZE above cap $MAX_IMAGE_BYTES"
      echo
      continue
    fi
    if dd if="$PATH_TO_READ" of="$RAW" bs=4194304 count="$MAX_IMAGE_BLOCKS" 2>>"$MANIFEST"; then
      if gzip -c "$RAW" > "$GZ" 2>>"$MANIFEST"; then
        rm -f "$RAW"
        echo "file=images/$OUT_BASE.gz"
        sha256sum "$GZ" 2>/dev/null || true
        HAS_IMAGE=1
      else
        echo "gzip_failed=1"
        rm -f "$RAW" "$GZ"
      fi
    else
      echo "dd_failed=1"
      rm -f "$RAW" "$GZ"
    fi
    echo
  } >> "$MANIFEST"
done
[ "$HAS_IMAGE" -eq 1 ] && exit 0 || exit 1
