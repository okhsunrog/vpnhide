package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.buildKernelPartitionMetadataCommand
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class KernelImageExportTest {
    @Test
    fun `metadata command reports kernel partitions without dumping vendor dlkm`() {
        val command = buildKernelPartitionMetadataCommand()

        assertTrue(command.contains("boot init_boot vendor_boot vendor_kernel_boot vendor_dlkm"))
        assertTrue(command.contains("sha256=skipped (vendor_dlkm"))
        assertTrue(command.contains("ro.boot.slot_suffix"))
        assertFalse(command.contains("dd if="))
    }
}
