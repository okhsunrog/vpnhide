package dev.okhsunrog.vpnhide.debug

import android.content.Context
import dev.okhsunrog.vpnhide.core.CoreDiagnostics
import dev.okhsunrog.vpnhide.core.CoreRoot
import kotlinx.coroutines.CancellationException
import java.io.File
import uniffi.vpnhide_core_ffi.CoreDiagnosticProbe as CoreProbe

internal data class JavaArtifacts(
    val entries: List<DiagnosticFileEntry>,
    val status: String,
) : AutoCloseable {
    override fun close() {
        entries.forEach { runCatching { it.file.delete() } }
        entries.mapNotNull { it.file.parentFile }.distinct().forEach { runCatching { it.delete() } }
    }
}

/** Called only by explicit export, separate from normal snapshots and hook hot paths. */
internal suspend fun collectJavaArtifacts(
    context: Context,
    includeFiles: Boolean,
): JavaArtifacts =
    try {
        CoreRoot.attach(context.applicationContext)
        when (val reply = CoreRoot.collect(listOf(CoreProbe.JavaHookArtifacts(includeFiles)))) {
            is CoreDiagnostics.Failed -> {
                JavaArtifacts(emptyList(), "Java collection unavailable: ${reply.status}: ${reply.detail.take(1024)}")
            }

            is CoreDiagnostics.Collected -> {
                val result = reply.results.single()
                val exportRoot = File(context.applicationInfo.dataDir, "vpnhide-core-export")
                val entries =
                    try {
                        javaArtifactEntries(exportRoot, result.artifacts)
                    } catch (e: Exception) {
                        discardJavaArtifacts(exportRoot, result.artifacts)
                        throw e
                    }
                JavaArtifacts(
                    entries,
                    "Java collection exit=${result.exit} truncated=${result.truncated}; inspect java-manifest.json for collected/skipped files and telemetry freshness.",
                )
            }
        }
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        JavaArtifacts(emptyList(), "Java collection incomplete: ${e.javaClass.simpleName}: ${e.message?.take(1024)}")
    }

/** A failed optional attachment must not discard the ordinary diagnostic report. */
internal fun writeDebugArchive(
    zip: File,
    text: Map<String, String>,
    files: List<DiagnosticFileEntry>,
    java: JavaArtifacts,
) {
    try {
        writeDiagnosticZip(
            zip,
            text + mapOf("collection-plan.txt" to java.status, "java/collection.txt" to java.status),
            files + java.entries,
        )
    } catch (e: Exception) {
        if (java.entries.isEmpty()) throw e
        writeDiagnosticZip(
            zip,
            text + ("collection-plan.txt" to java.status) +
                (
                    "java/collection.txt" to
                        "Java attachments omitted after packaging failure: ${e.javaClass.simpleName}: ${e.message?.take(1024)}"
                ),
            files,
        )
    }
}
