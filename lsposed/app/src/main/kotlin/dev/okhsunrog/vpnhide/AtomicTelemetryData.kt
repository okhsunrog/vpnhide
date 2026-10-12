package dev.okhsunrog.vpnhide

import java.io.File
import java.io.IOException

/** Same-directory rename only: a failed replacement leaves the old snapshot intact. */
internal fun writeAtomicTelemetry(
    destination: File,
    temporary: File,
    text: String,
    replace: (File, File) -> Boolean = { source, target -> source.renameTo(target) },
) {
    try {
        temporary.writeText(text)
        if (!replace(temporary, destination)) throw IOException("atomic telemetry replacement failed")
    } finally {
        temporary.delete()
    }
}
