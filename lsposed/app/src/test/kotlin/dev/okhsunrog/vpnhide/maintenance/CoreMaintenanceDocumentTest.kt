package dev.okhsunrog.vpnhide.maintenance

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.ByteArrayInputStream
import java.io.File

class CoreMaintenanceDocumentTest {
    @get:Rule val temp = TemporaryFolder()

    @Test
    fun `bounded document copy publishes only complete input and preserves previous bytes on refusal`() {
        val files = temp.newFolder()
        val hash = "a".repeat(64)
        val bytes = byteArrayOf(1, 2, 3)
        stageMaintenanceDocument(files, ByteArrayInputStream(bytes), hash, 3)
        val destination = File(files, "core-maintenance/$hash.zip")
        assertArrayEquals(bytes, destination.readBytes())
        for (input in listOf(byteArrayOf(4, 5), byteArrayOf(4, 5, 6, 7))) {
            assertThrows(IllegalStateException::class.java) { stageMaintenanceDocument(files, ByteArrayInputStream(input), hash, 3) }
            assertArrayEquals(bytes, destination.readBytes())
            assertEquals(listOf("$hash.zip"), requireNotNull(destination.parentFile?.list()).toList())
        }
    }
}
