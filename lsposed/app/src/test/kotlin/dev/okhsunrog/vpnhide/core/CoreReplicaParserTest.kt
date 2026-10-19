package dev.okhsunrog.vpnhide.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CoreReplicaParserTest {
    @Test
    fun currentFreshnessExposesAuthoritativeVpn() {
        val json =
            """
            {
              "config": {"settings":{}},
              "config_revision": 7,
              "operations_in_flight": [[3], [9]],
              "routing": {
                "freshness": "current",
                "generation": 4,
                "vpn_interfaces": ["tun0"],
                "last_good_vpn_interfaces": ["tun0"],
                "netlink_candidates": ["tun0", "ipsec0"]
              },
              "backend": {}
            }
            """.trimIndent()
        val view = CoreReplicaParser.parseSnapshot(json)
        assertEquals(7, view.configRevision)
        assertTrue(view.hasConfig)
        assertEquals(listOf(3L, 9L), view.operationsInFlight)
        assertEquals(CoreFreshness.Current, view.routing.freshness)
        assertEquals(listOf("tun0"), view.routing.authoritativeVpnInterfaces())
        assertEquals(listOf("tun0"), view.routing.displayVpnInterfaces())
    }

    @Test
    fun unknownFreshnessKeepsLastGoodDisplayOnly() {
        val json =
            """
            {
              "config": null,
              "config_revision": 2,
              "operations_in_flight": [],
              "routing": {
                "freshness": "unknown",
                "generation": 8,
                "vpn_interfaces": [],
                "last_good_vpn_interfaces": ["tun0"],
                "netlink_candidates": []
              },
              "backend": {}
            }
            """.trimIndent()
        val view = CoreReplicaParser.parseSnapshot(json)
        assertFalse(view.hasConfig)
        assertEquals(CoreFreshness.Unknown, view.routing.freshness)
        assertTrue(view.routing.authoritativeVpnInterfaces().isEmpty())
        assertEquals(listOf("tun0"), view.routing.displayVpnInterfaces())
    }
}
