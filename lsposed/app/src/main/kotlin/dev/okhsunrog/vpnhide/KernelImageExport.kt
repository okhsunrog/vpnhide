package dev.okhsunrog.vpnhide

import android.content.Context
import android.os.Process
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private const val KERNEL_PARTITION_METADATA_TIMEOUT_SEC: Long = 30
private const val KERNEL_IMAGE_EXPORT_TIMEOUT_SEC: Long = 180

internal fun buildKernelPartitionMetadataText(): String {
    val (exit, output) =
        suExec(
            buildKernelPartitionMetadataCommand(),
            timeoutSec = KERNEL_PARTITION_METADATA_TIMEOUT_SEC,
        )
    return buildString {
        appendLine("commandExit=$exit")
        appendLine(output.ifBlank { "(no output)" }.trimEnd())
    }.trimEnd()
}

internal suspend fun exportKernelImagesZip(context: Context): File? =
    withContext(Dispatchers.IO) {
        val appContext = context.applicationContext
        val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date())
        val partsDir = File(appContext.cacheDir, "vpnhide_kernel_images_${timestamp}_parts")
        val zipFile = File(appContext.cacheDir, "vpnhide_kernel_images_$timestamp.zip")
        try {
            runCatching { partsDir.deleteRecursively() }

            val (exit, output) =
                suExec(
                    buildKernelImagesExportCommand(
                        outputDir = partsDir.absolutePath,
                        appUid = Process.myUid(),
                    ),
                    timeoutSec = KERNEL_IMAGE_EXPORT_TIMEOUT_SEC,
                )

            val imageFiles =
                partsDir
                    .listFiles()
                    .orEmpty()
                    .filter { it.isFile && it.name.endsWith(".img.gz") }
                    .sortedBy { it.name }

            if (exit != 0 || imageFiles.isEmpty()) {
                HookLog.e("VpnHide: kernel image export failed: exit=$exit output=${output.take(400)}")
                return@withContext null
            }

            writeDiagnosticZip(
                zipFile = zipFile,
                textEntries = kernelExportTextEntries(appContext, partsDir, exit),
                fileEntries = imageFiles.map { file -> DiagnosticFileEntry("images/${file.name}", file) },
            )
            zipFile
        } catch (c: CancellationException) {
            throw c
        } catch (t: Throwable) {
            // Never let a zip/IO failure (e.g. /data full while packing tens of
            // MB of images) escape and strand the caller's UI in a stuck state.
            HookLog.e("VpnHide: kernel image export error: ${t.message}")
            runCatching { zipFile.delete() }
            null
        } finally {
            runCatching { partsDir.deleteRecursively() }
        }
    }

private fun kernelExportTextEntries(
    appContext: Context,
    partsDir: File,
    exit: Int,
): LinkedHashMap<String, String> {
    val manifest =
        partsDir
            .resolve("manifest.txt")
            .takeIf { it.isFile }
            ?.readText()
            .orEmpty()
    return linkedMapOf(
        "summary.txt" to
            buildString {
                appendLine("Kernel image export")
                appendLine("Generated: ${SimpleDateFormat("yyyy-MM-dd HH:mm:ss Z", Locale.US).format(Date())}")
                appendLine("App package: ${appContext.packageName}")
                appendLine("App version: ${appVersionText(appContext)}")
                appendLine("commandExit=$exit")
            }.trimEnd(),
        "manifest.txt" to manifest.ifBlank { "(missing manifest)" },
        "kernel_partitions.txt" to buildKernelPartitionMetadataText(),
    )
}

internal fun buildKernelPartitionMetadataCommand(): String =
    """
    resolve_partition() {
      NAME="${'$'}1"
      SLOT="${'$'}2"
      for CANDIDATE in "${'$'}NAME${'$'}SLOT" "${'$'}NAME"; do
        PATH_TO_READ="/dev/block/by-name/${'$'}CANDIDATE"
        if [ -e "${'$'}PATH_TO_READ" ]; then
          echo "${'$'}PATH_TO_READ"
          return 0
        fi
      done
      return 1
    }
    partition_metadata() {
      NAME="${'$'}1"
      echo "### ${'$'}NAME"
      PATH_TO_READ="${'$'}(resolve_partition "${'$'}NAME" "${'$'}SLOT_SUFFIX" 2>/dev/null || true)"
      if [ -z "${'$'}PATH_TO_READ" ]; then
        echo "present=0"
        echo
        return 0
      fi
      REAL_PATH="${'$'}(readlink -f "${'$'}PATH_TO_READ" 2>/dev/null || echo "${'$'}PATH_TO_READ")"
      SIZE="${'$'}(blockdev --getsize64 "${'$'}PATH_TO_READ" 2>/dev/null || stat -c %s "${'$'}PATH_TO_READ" 2>/dev/null || echo unknown)"
      echo "present=1"
      echo "path=${'$'}PATH_TO_READ"
      echo "real_path=${'$'}REAL_PATH"
      echo "size_bytes=${'$'}SIZE"
      ls -lZ "${'$'}PATH_TO_READ" 2>&1 || ls -l "${'$'}PATH_TO_READ" 2>&1 || true
      if [ "${'$'}NAME" = "vendor_dlkm" ]; then
        echo "sha256=skipped (vendor_dlkm can be large; export images separately only when needed)"
      elif [ "${'$'}SIZE" != "unknown" ] && [ "${'$'}SIZE" -le 268435456 ]; then
        sha256sum "${'$'}PATH_TO_READ" 2>&1 || true
      else
        echo "sha256=skipped (size above 268435456 bytes or unknown)"
      fi
      echo
    }

    SLOT_SUFFIX="${'$'}(getprop ro.boot.slot_suffix 2>/dev/null)"
    echo "slot_suffix=${'$'}SLOT_SUFFIX"
    echo "current_slot=${'$'}(bootctl get-current-slot 2>/dev/null || true)"
    echo "current_slot_suffix=${'$'}(bootctl get-suffix "${'$'}(bootctl get-current-slot 2>/dev/null)" 2>/dev/null || true)"
    for PROP in \
      ro.boot.slot_suffix ro.boot.slot ro.boot.bootdevice ro.boot.super_partition \
      ro.boot.init_boot ro.boot.vendor_boot ro.boot.vbmeta.device_state \
      ro.boot.verifiedbootstate ro.boot.dynamic_partitions
    do
      VALUE="${'$'}(getprop "${'$'}PROP" 2>/dev/null)"
      [ -n "${'$'}VALUE" ] && printf '%s=%s\n' "${'$'}PROP" "${'$'}VALUE"
    done
    echo
    for NAME in boot init_boot vendor_boot vendor_kernel_boot vendor_dlkm; do
      partition_metadata "${'$'}NAME"
    done
    echo "### by-name entries"
    ls -lZ /dev/block/by-name 2>&1 | grep -E " boot|init_boot|vendor_boot|vendor_kernel_boot|vendor_dlkm" ||
      ls -l /dev/block/by-name 2>&1 | grep -E " boot|init_boot|vendor_boot|vendor_kernel_boot|vendor_dlkm" || true
    """.trimIndent()

// Long because this is one root-side shell script kept as a single template so
// the app writes one manifest and preserves command ordering.
@Suppress("LongMethod")
private fun buildKernelImagesExportCommand(
    outputDir: String,
    appUid: Int,
): String {
    val quotedDir = shellQuote(outputDir)
    return """
        set +e
        OUT_DIR=$quotedDir
        APP_UID=$appUid
        # Kernel/boot images are tens of MB; cap the copy so an unexpectedly
        # large or mislabelled partition can't fill /data. 512 MiB = 128 * 4 MiB.
        MAX_IMAGE_BYTES=536870912
        MAX_IMAGE_BLOCKS=128
        rm -rf "${'$'}OUT_DIR"
        mkdir -p "${'$'}OUT_DIR" || exit 1
        MANIFEST="${'$'}OUT_DIR/manifest.txt"
        SLOT_SUFFIX="${'$'}(getprop ro.boot.slot_suffix 2>/dev/null)"
        {
          echo "schema=1"
          echo "slot_suffix=${'$'}SLOT_SUFFIX"
          echo "created_utc=${'$'}(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date)"
          echo "included=boot init_boot vendor_boot vendor_kernel_boot"
          echo "skipped=vendor_dlkm (metadata only; usually large and not a kernel image)"
          echo
        } > "${'$'}MANIFEST"

        resolve_partition() {
          NAME="${'$'}1"
          for CANDIDATE in "${'$'}NAME${'$'}SLOT_SUFFIX" "${'$'}NAME"; do
            PATH_TO_READ="/dev/block/by-name/${'$'}CANDIDATE"
            if [ -e "${'$'}PATH_TO_READ" ]; then
              echo "${'$'}PATH_TO_READ"
              return 0
            fi
          done
          return 1
        }

        HAS_IMAGE=0
        for NAME in boot init_boot vendor_boot vendor_kernel_boot; do
          PATH_TO_READ="${'$'}(resolve_partition "${'$'}NAME" 2>/dev/null || true)"
          {
            echo "### ${'$'}NAME"
            if [ -z "${'$'}PATH_TO_READ" ]; then
              echo "present=0"
              echo
              continue
            fi
            SIZE="${'$'}(blockdev --getsize64 "${'$'}PATH_TO_READ" 2>/dev/null || echo unknown)"
            OUT_BASE="${'$'}NAME${'$'}SLOT_SUFFIX.img"
            RAW="${'$'}OUT_DIR/${'$'}OUT_BASE"
            GZ="${'$'}OUT_DIR/${'$'}OUT_BASE.gz"
            echo "present=1"
            echo "path=${'$'}PATH_TO_READ"
            echo "size_bytes=${'$'}SIZE"
            case "${'$'}SIZE" in
              ''|*[!0-9]*)
                echo "skipped=size unknown (refusing unbounded dd)"
                echo
                continue
                ;;
            esac
            if [ "${'$'}SIZE" -gt "${'$'}MAX_IMAGE_BYTES" ]; then
              echo "skipped=size ${'$'}SIZE above cap ${'$'}MAX_IMAGE_BYTES"
              echo
              continue
            fi
            if dd if="${'$'}PATH_TO_READ" of="${'$'}RAW" bs=4194304 count="${'$'}MAX_IMAGE_BLOCKS" 2>>"${'$'}MANIFEST"; then
              if gzip -c "${'$'}RAW" > "${'$'}GZ" 2>>"${'$'}MANIFEST"; then
                rm -f "${'$'}RAW"
                echo "file=images/${'$'}OUT_BASE.gz"
                sha256sum "${'$'}GZ" 2>/dev/null || true
                HAS_IMAGE=1
              else
                echo "gzip_failed=1"
                rm -f "${'$'}RAW" "${'$'}GZ"
              fi
            else
              echo "dd_failed=1"
              rm -f "${'$'}RAW" "${'$'}GZ"
            fi
            echo
          } >> "${'$'}MANIFEST"
        done
        chmod 0700 "${'$'}OUT_DIR" 2>/dev/null || true
        chmod 0600 "${'$'}OUT_DIR"/* 2>/dev/null || true
        chown -R "${'$'}APP_UID:${'$'}APP_UID" "${'$'}OUT_DIR" 2>/dev/null || true
        [ "${'$'}HAS_IMAGE" -eq 1 ] && exit 0 || exit 1
        """.trimIndent()
}

private fun shellQuote(value: String): String = "'" + value.replace("'", "'\"'\"'") + "'"
