// AUTO-GENERATED from data/interfaces.toml — do not edit by hand. Regenerate with: python3 scripts/codegen-interfaces.py

package dev.okhsunrog.vpnhide.generated

import org.junit.Assert.assertEquals
import org.junit.Test

class IfaceListsGeneratedTest {
    @Test
    fun `generated vectors`() {
        assertEquals("v4-rmnet0", true, IfaceLists.isNeverHide("v4-rmnet0"))
        assertEquals("v4-rmnet_data0", true, IfaceLists.isNeverHide("v4-rmnet_data0"))
        assertEquals("v4-wlan0", true, IfaceLists.isNeverHide("v4-wlan0"))
        assertEquals("v4-x", true, IfaceLists.isNeverHide("v4-x"))
        assertEquals("thread-wpan", true, IfaceLists.isNeverHide("thread-wpan"))
        assertEquals("Thread-Wpan", true, IfaceLists.isNeverHide("Thread-Wpan"))
        assertEquals("sit0", true, IfaceLists.isNeverHide("sit0"))
        assertEquals("tunl0", true, IfaceLists.isNeverHide("tunl0"))
        assertEquals("ip6tnl0", true, IfaceLists.isNeverHide("ip6tnl0"))
        assertEquals("ip_vti0", true, IfaceLists.isNeverHide("ip_vti0"))
        assertEquals("ip6_vti0", true, IfaceLists.isNeverHide("ip6_vti0"))
        assertEquals("gre0", true, IfaceLists.isNeverHide("gre0"))
        assertEquals("ipsec250", true, IfaceLists.isNeverHide("ipsec250"))
        assertEquals("IPSec250", true, IfaceLists.isNeverHide("IPSec250"))
        assertEquals("v4-", false, IfaceLists.isNeverHide("v4-"))
        assertEquals("v4", false, IfaceLists.isNeverHide("v4"))
        assertEquals("tun0", false, IfaceLists.isNeverHide("tun0"))
        assertEquals("wg0", false, IfaceLists.isNeverHide("wg0"))
        assertEquals("wlan0", false, IfaceLists.isNeverHide("wlan0"))
        assertEquals("thread-wpan-extra", false, IfaceLists.isNeverHide("thread-wpan-extra"))
        assertEquals("if33", false, IfaceLists.isNeverHide("if33"))
        assertEquals("sit1", false, IfaceLists.isNeverHide("sit1"))
        assertEquals("tunl1", false, IfaceLists.isNeverHide("tunl1"))
        assertEquals("ip6tnl1", false, IfaceLists.isNeverHide("ip6tnl1"))
        assertEquals("ip_vti1", false, IfaceLists.isNeverHide("ip_vti1"))
        assertEquals("ip6_vti1", false, IfaceLists.isNeverHide("ip6_vti1"))
        assertEquals("gre1", false, IfaceLists.isNeverHide("gre1"))
        assertEquals("ipsec0", false, IfaceLists.isNeverHide("ipsec0"))
        assertEquals("ipsec1", false, IfaceLists.isNeverHide("ipsec1"))
        assertEquals("ipsec1234", false, IfaceLists.isNeverHide("ipsec1234"))
        assertEquals("", false, IfaceLists.isNeverHide(""))
    }
}
