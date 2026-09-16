package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.vpnPollFingerprint
import dev.okhsunrog.vpnhide.diagnostics.vpnPollNeedsRefresh
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class VpnStatePollerTest {
    private fun snapshot(vpn: Boolean = false) =
        mapOf(
            "vpn_networks" to "Current Networks:\n",
            "vpn_ifaces" to if (vpn) "wg0=unknown" else "",
            "vpn_routes4" to if (vpn) "default dev wg0 table 51820\nprobe_ok" else "probe_ok",
            "vpn_routes6" to "probe_ok",
            "vpn_rules4" to "0: from all lookup local\nprobe_ok",
            "vpn_rules6" to "probe_ok",
        )

    @Test
    fun `both VPN edges trigger and stable observations stay quiet`() {
        val off = vpnPollFingerprint(snapshot())
        val on = vpnPollFingerprint(snapshot(true))
        assertTrue(vpnPollNeedsRefresh(null, off, false))
        assertTrue(vpnPollNeedsRefresh(off, on, false))
        assertTrue(vpnPollNeedsRefresh(on, off, false))
        assertFalse(vpnPollNeedsRefresh(on, on, false))
    }

    @Test
    fun `failure and recovery refresh once without interpreting failure as off`() {
        val on = vpnPollFingerprint(snapshot(true))
        assertTrue(vpnPollNeedsRefresh(on, null, false))
        assertFalse(vpnPollNeedsRefresh(null, null, true))
        assertTrue(vpnPollNeedsRefresh(null, on, true))
    }

    @Test
    fun `policy changes are detected even when VPN interface is unchanged`() {
        val before = snapshot(true)
        val after = before + ("vpn_rules4" to "100: uidrange 10400-10400 lookup 51820\nprobe_ok")
        assertNotEquals(vpnPollFingerprint(before), vpnPollFingerprint(after))
    }

    @Test
    fun `rule ordering changes remain observable`() {
        val first = snapshot() + ("vpn_rules4" to "a\nb\nprobe_ok")
        val second = snapshot() + ("vpn_rules4" to "b\na\nprobe_ok")
        assertNotEquals(vpnPollFingerprint(first), vpnPollFingerprint(second))
    }

    @Test(expected = IllegalArgumentException::class)
    fun `missing routing evidence is not a VPN off sample`() {
        vpnPollFingerprint(snapshot() - "vpn_rules6")
    }

    @Test
    fun `route lifetime countdown does not look like a network transition`() {
        val first = snapshot() + ("vpn_routes6" to "default via fe80::1 dev wlan0 expires 5000sec\nprobe_ok")
        val later = snapshot() + ("vpn_routes6" to "default via fe80::1 dev wlan0 expires 4996sec\nprobe_ok")
        assertEquals(vpnPollFingerprint(first), vpnPollFingerprint(later))
        assertNotEquals(vpnPollFingerprint(first), vpnPollFingerprint(snapshot()))
    }

    @Test
    fun `network only shell omits inventory and backend probes`() {
        val command = buildRootShellSnapshotCommand(networkOnly = true)
        val process = ProcessBuilder("sh", "-c", command).start()
        val sections = parseRootShellSnapshot(process.inputStream.bufferedReader().readText()) { _, _ -> }
        assertEquals(0, process.waitFor())
        assertEquals(setOf("vpn_ifaces", "vpn_networks", "vpn_routes4", "vpn_routes6", "vpn_rules4", "vpn_rules6"), sections.keys)
    }
}
