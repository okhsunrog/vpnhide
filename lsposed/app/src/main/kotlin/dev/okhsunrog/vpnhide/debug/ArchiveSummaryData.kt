package dev.okhsunrog.vpnhide.debug

import org.json.JSONObject
import java.io.File
import java.util.zip.ZipFile

internal data class ArchiveSummaryData(
    val kernelImages: Int,
    val frameworkFiles: Int,
    val javaSkipped: Int,
)

internal fun archiveSummary(file: File): ArchiveSummaryData =
    ZipFile(file).use { zip ->
        val names =
            zip
                .entries()
                .asSequence()
                .map { it.name }
                .toList()
        val manifest = zip.getEntry("java/java-manifest.json")
        val skipped =
            runCatching {
                if (manifest == null || manifest.size !in 0..JAVA_MANIFEST_BYTES) return@runCatching 0
                val entries = zip.getInputStream(manifest).bufferedReader().use { JSONObject(it.readText()).getJSONArray("entries") }
                (0 until entries.length()).count { entries.getJSONObject(it).has("skipped") }
            }.getOrDefault(0)
        ArchiveSummaryData(
            names.count {
                it.startsWith("images/") && it.endsWith(".img.gz")
            },
            names.count { it.startsWith("java/framework-") },
            skipped,
        )
    }
