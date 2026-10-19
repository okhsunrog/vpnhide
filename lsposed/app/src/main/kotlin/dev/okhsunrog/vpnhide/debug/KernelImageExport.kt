package dev.okhsunrog.vpnhide.debug

import android.content.Context
import dev.okhsunrog.vpnhide.core.CoreProbe
import dev.okhsunrog.vpnhide.core.CoreProbeResult
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.hook.HookLog
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

internal fun buildKernelPartitionMetadataText(): String =
    kernelPartitionMetadataText(CoreRoot.collectOneBlocking(CoreProbe.KernelPartitions))

private fun kernelPartitionMetadataText(result: CoreProbeResult): String =
    buildString {
        appendLine("commandExit=${result.exit}")
        appendLine(result.text.ifBlank { "(no output)" }.trimEnd())
    }.trimEnd()

/**
 * Capture the active boot/kernel partition images and pack them with the caller's
 * already-built [stateJson] into one zip, so a kernel bug report travels as a single
 * self-contained file. Core reads the partitions (`KernelImages`, `KernelPartitions`)
 * and hands the gzipped images to the app in its export dir. If image capture yields
 * nothing, the zip still carries the state.json + partition metadata.
 */
internal suspend fun writeKernelBundleZip(
    context: Context,
    timestamp: String,
    stateJson: String,
): File? =
    withContext(Dispatchers.IO) {
        val appContext = context.applicationContext
        CoreRoot.attach(appContext)
        val zipFile = File(appContext.cacheDir, "vpnhide_debug_$timestamp.zip")
        var artifacts: List<File> = emptyList()
        try {
            val (images, partitions) =
                CoreRoot.collect(listOf(CoreProbe.KernelImages, CoreProbe.KernelPartitions)).results(2)
            artifacts = images.artifacts
            val imageFiles = artifacts.filter { it.isFile && it.name.endsWith(".img.gz") }.sortedBy { it.name }
            if (imageFiles.isEmpty()) {
                HookLog.e("VpnHide: kernel image capture produced no images: exit=${images.exit} output=${images.text.take(400)}")
            }
            val manifest =
                artifacts
                    .firstOrNull { it.name == "manifest.txt" && it.isFile }
                    ?.readText()
                    .orEmpty()
            writeDiagnosticZip(
                zipFile = zipFile,
                textEntries =
                    mapOf(
                        "state.json" to stateJson,
                        "manifest.txt" to manifest.ifBlank { "(missing manifest)" },
                        "kernel_partitions.txt" to kernelPartitionMetadataText(partitions),
                    ),
                fileEntries = imageFiles.map { file -> DiagnosticFileEntry("images/${file.name}", file) },
            )
            zipFile
        } catch (c: CancellationException) {
            throw c
        } catch (t: Throwable) {
            // Never let a zip/IO failure (e.g. /data full while packing tens of MB of
            // images) escape and strand the caller's UI in a stuck state.
            HookLog.e("VpnHide: kernel bundle export error: ${t.message}")
            runCatching { zipFile.delete() }
            null
        } finally {
            // The images are tens of MB: drop core's copies once packed.
            artifacts.forEach { runCatching { it.delete() } }
        }
    }
