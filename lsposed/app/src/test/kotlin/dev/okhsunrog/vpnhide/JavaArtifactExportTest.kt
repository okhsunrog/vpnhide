package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.*
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.nio.file.Files
import java.util.zip.ZipFile

class JavaArtifactExportTest {
    @Test fun `ordinary archive preserves baseline when Java attachment disappears`() {
        val root = Files.createTempDirectory("java-export").toFile()
        try {
            val zip = File(root, "report.zip")
            writeDebugArchive(
                zip,
                mapOf("state.json" to "{}"),
                emptyList(),
                JavaArtifacts(listOf(DiagnosticFileEntry("java/framework-0.jar", File(root, "missing"))), "planned"),
            )
            ZipFile(zip).use { archive ->
                assertNotNull(archive.getEntry("state.json"))
                assertNull(archive.getEntry("java/framework-0.jar"))
                assertTrue(
                    archive
                        .getInputStream(archive.getEntry("java/collection.txt"))
                        .bufferedReader()
                        .readText()
                        .contains("omitted"),
                )
            }
            assertFalse(root.listFiles().orEmpty().any { it.name.endsWith(".tmp") })
        } finally {
            root.deleteRecursively()
        }
    }

    @Test fun `valid artifacts package with manifest and cleanup only owned files`() {
        val root = Files.createTempDirectory("java-export").toFile()
        try {
            val export = File(root, "exports").apply { mkdir() }
            val directory = File(export, "java-hooks-1").apply { mkdir() }
            val jar = File(directory, "framework-0.jar").apply { writeText("bytes") }
            val manifest = File(directory, "java-manifest.json").apply { writeText("{\"entries\":[{\"skipped\":\"missing\"}]}") }
            val files = javaArtifactEntries(export, listOf(jar, manifest))
            val zip = File(root, "report.zip")
            JavaArtifacts(files, "collected").use { writeDebugArchive(zip, mapOf("state.json" to "{}"), emptyList(), it) }
            assertEquals(ArchiveSummaryData(0, 1, 1), archiveSummary(zip))
            assertFalse(jar.exists())
            val outside = File(root, "private.jar").apply { writeText("keep") }
            discardJavaArtifacts(export, listOf(outside))
            assertEquals("keep", outside.readText())
        } finally {
            root.deleteRecursively()
        }
    }

    @Test fun `redirected untrusted oversized and missing manifest artifacts are refused`() {
        val root = Files.createTempDirectory("java-export").toFile()
        try {
            val directory = File(root, "java-hooks-1").apply { mkdir() }
            val jar = File(directory, "framework-0.jar").apply { writeText("x") }
            assertThrows(IllegalArgumentException::class.java) { javaArtifactEntries(root, listOf(jar)) }
            val manifest = File(directory, "java-manifest.json").apply { writeText("{}") }
            val link = File(directory, "framework-1.jar")
            Files.createSymbolicLink(link.toPath(), jar.toPath())
            assertThrows(IllegalArgumentException::class.java) { javaArtifactEntries(root, listOf(link, manifest)) }
            java.io.RandomAccessFile(jar, "rw").use { it.setLength(65L shl 20) }
            assertThrows(IllegalArgumentException::class.java) { javaArtifactEntries(root, listOf(jar, manifest)) }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test fun `archive failure is atomic preserves previous result and rejects path traversal`() {
        val root = Files.createTempDirectory("java-export").toFile()
        try {
            val zip = File(root, "report.zip").apply { writeText("previous") }
            assertThrows(Exception::class.java) {
                writeDiagnosticZip(zip, mapOf("state.json" to "{}"), listOf(DiagnosticFileEntry("missing", File(root, "absent"))))
            }
            assertEquals("previous", zip.readText())
            assertThrows(IllegalArgumentException::class.java) { writeDiagnosticZip(zip, mapOf("../outside" to "x")) }
            assertFalse(root.listFiles().orEmpty().any { it.name.endsWith(".tmp") })
        } finally {
            root.deleteRecursively()
        }
    }

    @Test fun `packaging deadline and cancellation cleanup preserve baseline resources`() {
        val root = Files.createTempDirectory("java-export").toFile()
        try {
            val zip = File(root, "report.zip").apply { writeText("previous") }
            assertThrows(IllegalStateException::class.java) {
                writeDiagnosticZip(zip, mapOf("state.json" to "{}"), timeBudgetMs = 0)
            }
            assertEquals("previous", zip.readText())
            val source = File(root, "framework-0.jar").apply { writeText("fixture") }
            assertThrows(kotlinx.coroutines.CancellationException::class.java) {
                JavaArtifacts(listOf(DiagnosticFileEntry("java/framework-0.jar", source)), "test").use {
                    throw kotlinx.coroutines.CancellationException("cancelled")
                }
            }
            assertFalse(source.exists())
            assertFalse(root.listFiles().orEmpty().any { it.name.endsWith(".tmp") })
        } finally {
            root.deleteRecursively()
        }
    }
}
