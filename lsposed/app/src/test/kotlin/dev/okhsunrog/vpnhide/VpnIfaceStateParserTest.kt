package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Test

class VpnIfaceStateParserTest {
    @Test
    fun `parser accepts grep operstate output`() {
        val raw =
            """
            /sys/class/net/wlan0/operstate:up
            /sys/class/net/tun0/operstate:unknown
            /sys/class/net/rmnet_data0/operstate:down
            """.trimIndent()

        assertEquals(listOf("tun0" to "unknown"), parseVpnIfaceStates(raw))
    }

    @Test
    fun `parser keeps compatibility with legacy iface equals state output`() {
        val raw =
            """
            wlan0=up
            wg-client=up
            rmnet_data0=down
            """.trimIndent()

        assertEquals(listOf("wg-client" to "up"), parseVpnIfaceStates(raw))
    }
}
