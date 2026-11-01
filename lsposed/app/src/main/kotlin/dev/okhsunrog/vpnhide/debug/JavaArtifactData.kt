package dev.okhsunrog.vpnhide.debug

import java.io.File
import java.nio.file.Files

internal const val JAVA_ARTIFACT_BYTES = 96L * 1024 * 1024
internal const val JAVA_MANIFEST_BYTES = 256L * 1024

/** Validate again at the app boundary; never package/delete an arbitrary returned path. */
internal fun javaArtifactEntries(
    exportRoot: File,
    artifacts: List<File>,
    requireManifest: Boolean = true,
): List<DiagnosticFileEntry> {
    require(artifacts.size <= 13) { "too many Java artifacts" }
    val root = exportRoot.canonicalFile
    var total = 0L
    return artifacts
        .map { file ->
            require(!Files.isSymbolicLink(file.toPath()) && file.isFile) { "Java artifact is not a regular file" }
            val canonical = file.canonicalFile
            val parent = requireNotNull(canonical.parentFile)
            require(parent.parentFile == root && parent.name.startsWith("java-hooks-")) { "Java artifact outside export directory" }
            require(file.absoluteFile == canonical) { "redirected Java artifact" }
            val manifest = file.name == "java-manifest.json"
            require(manifest || Regex("framework-[0-9]+\\.(jar|apk)").matches(file.name)) { "unexpected Java artifact name" }
            val limit = if (manifest) JAVA_MANIFEST_BYTES else 64L * 1024 * 1024
            require(file.length() <= limit) { "Java artifact exceeds file limit" }
            if (!manifest) total += file.length()
            require(total <= JAVA_ARTIFACT_BYTES) { "Java artifact total exceeds limit" }
            DiagnosticFileEntry("java/${file.name}", file)
        }.also { entries ->
            require(entries.map { it.name }.distinct().size == entries.size) { "duplicate Java artifacts" }
            require(!requireManifest || entries.any { it.name == "java/java-manifest.json" }) { "Java manifest missing" }
        }
}

internal fun discardJavaArtifacts(
    exportRoot: File,
    files: List<File>,
) {
    files.take(13).forEach { file ->
        runCatching {
            javaArtifactEntries(exportRoot, listOf(file), requireManifest = false).forEach { it.file.delete() }
        }
    }
}
