package dev.okhsunrog.vpnhide

import java.io.File

/**
 * Core's diagnostics probe source (core-daemon `src/device_probes`, Rust: core runs no
 * shell). Core's own tests assert the emitted sections against this app's parsers
 * (`REQUIRED_ROOT_SNAPSHOT_SECTIONS`); these helpers pin the fixed paths both sides share.
 */
internal fun coreProbeSource(name: String): String = repoFile("crates/core-daemon/src/device_probes/$name")

private fun repoFile(path: String): String {
    var dir: File? = File("").absoluteFile
    while (dir != null) {
        val file = File(dir, path)
        if (file.isFile) return file.readText()
        dir = dir.parentFile
    }
    error("$path not found above ${File("").absolutePath}")
}
