package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class VpnPresenceDataTest {
    private val ims =
        "  NetworkAgentInfo{ni{MOBILE[IWLAN] CONNECTED extra: ims} lp{InterfaceName: ipsec1} " +
            "nc{[ Transports: CELLULAR Capabilities: IMS&MMTEL&NOT_VPN ]}}"
    private val vpn =
        "  NetworkAgentInfo{ni{VPN CONNECTED} lp{InterfaceName: ipsec2} " +
            "nc{[ Transports: VPN Capabilities: INTERNET&TRUSTED ]}}"

    private fun snapshot(
        networks: String = ims,
        ifaces: String = "ipsec1=unknown",
        routes: String = "default dev ipsec1 table 104",
    ) = mapOf(
        "vpn_networks" to "Current Networks:\n$networks\n",
        "vpn_ifaces" to ifaces,
        "vpn_routes4" to "$routes\nprobe_ok",
        "vpn_routes6" to "probe_ok",
    )

    @Test
    fun `IMS tunnel does not activate VPN diagnostics`() {
        assertTrue(vpnPresenceFromSnapshot(snapshot()).interfaces.isEmpty())
    }

    @Test
    fun `real IPsec VPN alongside IMS stays detectable`() {
        assertEquals(setOf("ipsec2"), vpnPresenceFromSnapshot(snapshot("$ims\n$vpn")).interfaces)
    }

    @Test
    fun `requests and history cannot create an active VPN`() {
        val networks = "$ims\n    NetworkRequest [ Transports: VPN ]\nHistorical networks:\n$vpn"
        assertTrue(vpnPresenceFromSnapshot(snapshot(networks)).interfaces.isEmpty())
    }

    @Test
    fun `root WireGuard requires an up interface and usable route`() {
        val root = snapshot(ifaces = "wg0=unknown", routes = "default dev wg0 table 51820")
        assertEquals(setOf("wg0"), vpnPresenceFromSnapshot(root).interfaces)
        assertTrue(vpnPresenceFromSnapshot(root + ("vpn_routes4" to "local 10.0.0.1 dev wg0 table local\nprobe_ok")).interfaces.isEmpty())
        assertTrue(vpnPresenceFromSnapshot(root + ("vpn_ifaces" to "wg0=down")).interfaces.isEmpty())
    }

    @Test(expected = IllegalArgumentException::class)
    fun `failed framework probe is not VPN off`() {
        vpnPresenceFromSnapshot(snapshot() + ("vpn_networks" to "Permission Denial"))
    }

    @Test
    fun `root tunnel lookup uses caller UID without sending packets`() {
        val command = tunnelRouteProbeCommand(snapshot(routes = "default dev wg0 table 51820"), setOf("wg0"), 10402)
        assertTrue(command.contains("ip -4 route get '1.1.1.1' uid 10402"))
        assertEquals(true, tunnelRouteProbeResult("1.1.1.1 dev wg0 table 51820 uid 10402", setOf("wg0")))
        assertEquals(false, tunnelRouteProbeResult("1.1.1.1 via 192.0.2.1 dev wlan0 uid 10402", setOf("wg0")))
        assertNull(tunnelRouteProbeResult("probe_error", setOf("wg0")))
    }

    @Test
    fun `partial root tunnel checks its routed prefix and handles IPv6`() {
        val sections = snapshot(routes = "10.8.0.0/16 dev wg0") + ("vpn_routes6" to "default dev wg0 table 51820")
        val command = tunnelRouteProbeCommand(sections, setOf("wg0"), 10402)
        assertTrue(command.contains("ip -4 route get '10.8.0.0' uid 10402"))
        assertTrue(command.contains("ip -6 route get '2606:4700:4700::1111' uid 10402"))
        assertFalse(command.contains("ping"))
    }
}
