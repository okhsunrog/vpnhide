package dev.okhsunrog.vpnhide.debug

import java.io.File
import java.io.FileOutputStream
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/** Atomic private packaging; limits refer to uncompressed input (also bounds ZIP size). */
internal fun writeDiagnosticZip(
    zipFile: File,
    textEntries: Map<String, String>,
    fileEntries: List<DiagnosticFileEntry> = emptyList(),
    timeBudgetMs: Long = 180_000,
) {
    val limit = if (fileEntries.any { it.name.startsWith("images/") }) 2200L shl 20 else 128L shl 20
    require(textEntries.values.sumOf { it.toByteArray().size.toLong() } <= 16L shl 20) { "diagnostic text budget exceeded" }
    val names = textEntries.keys + fileEntries.map { it.name }
    require(
        names.size == names.distinct().size &&
            names.all { !it.startsWith('/') && it.split('/').none { part -> part == ".." || part.isEmpty() } },
    ) { "invalid archive names" }
    val stage = File.createTempFile(".vpnhide-zip-", ".tmp", zipFile.parentFile)
    try {
        FileOutputStream(stage).use { output ->
            ZipOutputStream(output).use { zip ->
                val writer = BoundedZipWriter(zip, limit, System.nanoTime() + timeBudgetMs * 1_000_000)
                textEntries.forEach { (name, text) -> writer.add(name, text.byteInputStream()) }
                fileEntries.forEach { writer.add(it.name, it.file.inputStream()) }
                zip.finish()
                zip.flush()
                output.fd.sync()
            }
        }
        check(stage.length() <= limit) { "compressed archive budget exceeded" }
        Files.move(stage.toPath(), zipFile.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
    } finally {
        stage.delete()
    }
}

private class BoundedZipWriter(
    private val zip: ZipOutputStream,
    private val limit: Long,
    private val deadline: Long,
) {
    private var used = 0L

    fun add(
        name: String,
        input: java.io.InputStream,
    ) {
        input.use {
            zip.putNextEntry(ZipEntry(name))
            val buffer = ByteArray(65536)
            while (true) {
                check(System.nanoTime() < deadline) { "archive packaging deadline exceeded" }
                val count = it.read(buffer)
                if (count < 0) break
                used += count
                check(used <= limit) { "diagnostic archive budget exceeded" }
                zip.write(buffer, 0, count)
            }
            zip.closeEntry()
        }
    }
}
