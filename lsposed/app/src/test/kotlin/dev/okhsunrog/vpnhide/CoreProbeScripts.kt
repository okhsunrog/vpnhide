package dev.okhsunrog.vpnhide

import java.io.File

/**
 * A root probe script as core runs it (core-daemon `probes/shell`). The app parses their
 * output, so its tests pin the sections and markers the scripts emit.
 */
internal fun coreProbeScript(name: String): String = repoFile("crates/core-daemon/probes/shell/$name")

/** core-daemon `diagnostics.rs`, which supplies the scripts' paths. */
internal fun coreDiagnosticsSource(): String = repoFile("crates/core-daemon/src/diagnostics.rs")

private fun repoFile(path: String): String {
    var dir: File? = File("").absoluteFile
    while (dir != null) {
        val file = File(dir, path)
        if (file.isFile) return file.readText()
        dir = dir.parentFile
    }
    error("$path not found above ${File("").absolutePath}")
}
