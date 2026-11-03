package dev.okhsunrog.vpnhide.maintenance

import java.io.File
import java.io.InputStream

/** Bounded SAF copy. Authentication and archive validation happen in Core. */
internal fun stageMaintenanceDocument(
    files: File,
    input: InputStream,
    hash: String,
    expectedBytes: Long,
): File {
    val directory = File(files, "core-maintenance").also { check(it.mkdirs() || it.isDirectory) }
    val temporary = File.createTempFile("copy-", ".zip", directory)
    try {
        temporary.outputStream().use { output ->
            var remaining = expectedBytes
            val block = ByteArray(64 * 1024)
            while (remaining > 0) {
                val n = input.read(block, 0, minOf(remaining, block.size.toLong()).toInt())
                check(n > 0)
                output.write(block, 0, n)
                remaining -= n
            }
            check(input.read() == -1)
            output.fd.sync()
        }
        val destination = File(directory, "$hash.zip")
        check(temporary.renameTo(destination))
        return destination
    } finally {
        temporary.delete()
    }
}
