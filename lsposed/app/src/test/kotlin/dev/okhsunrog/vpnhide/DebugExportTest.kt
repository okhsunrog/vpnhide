package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.buildBootLsposedLogcatCommand
import org.junit.Assert.assertTrue
import org.junit.Test

class DebugExportTest {
    @Test
    fun `boot lsposed logcat command captures current boot ring buffer context`() {
        val command = buildBootLsposedLogcatCommand()

        assertTrue(command.contains("logcat -d -b all -v threadtime"))
        assertTrue(command.contains("VpnHide-LSPosed"))
        assertTrue(command.contains("LSPosed-Bridge"))
        assertTrue(command.contains("VectorNative"))
        assertTrue(command.contains("VectorBridge"))
        assertTrue(command.contains("LSPosedService"))
        assertTrue(command.contains("LSPlt"))
        assertTrue(command.contains("tail -2000"))
    }
}
