package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import java.io.IOException
import java.nio.file.Files

class AtomicTelemetryTest {
    @Test
    fun `reader opened before replacement retains complete old snapshot`() {
        val directory = Files.createTempDirectory("telemetry-reader").toFile()
        try {
            val target = directory.resolve("state").apply { writeText("old complete snapshot") }
            target.inputStream().bufferedReader().use { reader ->
                writeAtomicTelemetry(target, directory.resolve("state.tmp"), "new complete snapshot")
                assertEquals("old complete snapshot", reader.readText())
                assertEquals("new complete snapshot", target.readText())
            }
        } finally {
            directory.deleteRecursively()
        }
    }

    @Test
    fun `failed rename preserves old snapshot and following write recovers`() {
        val directory = Files.createTempDirectory("telemetry").toFile()
        try {
            val target = directory.resolve("state").apply { writeText("old complete snapshot") }
            val temporary = directory.resolve("state.tmp")
            try {
                writeAtomicTelemetry(target, temporary, "new complete snapshot") { _, destination ->
                    assertEquals("old complete snapshot", destination.readText())
                    false
                }
                throw AssertionError("replacement failure must be reported")
            } catch (_: IOException) {
                assertEquals("old complete snapshot", target.readText())
                assertFalse(temporary.exists())
            }
            writeAtomicTelemetry(target, temporary, "new complete snapshot")
            assertEquals("new complete snapshot", target.readText())
            assertFalse(temporary.exists())
        } finally {
            directory.deleteRecursively()
        }
    }
}
