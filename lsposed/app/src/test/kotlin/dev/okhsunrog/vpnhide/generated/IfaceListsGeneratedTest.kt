// AUTO-GENERATED from data/interfaces.toml — do not edit by hand. Regenerate with: python3 scripts/codegen-interfaces.py

package dev.okhsunrog.vpnhide.generated

import org.junit.Assert.assertEquals
import org.junit.Test

class IfaceListsGeneratedTest {
    @Test
    fun `generated vectors`() {
        assertEquals("tun0", true, IfaceLists.isVpnIface("tun0"))
        assertEquals("tun", true, IfaceLists.isVpnIface("tun"))
        assertEquals("tun1234", true, IfaceLists.isVpnIface("tun1234"))
        assertEquals("tap0", true, IfaceLists.isVpnIface("tap0"))
        assertEquals("wg0", true, IfaceLists.isVpnIface("wg0"))
        assertEquals("wg-client", true, IfaceLists.isVpnIface("wg-client"))
        assertEquals("ppp0", true, IfaceLists.isVpnIface("ppp0"))
        assertEquals("ipsec0", true, IfaceLists.isVpnIface("ipsec0"))
        assertEquals("xfrm0", true, IfaceLists.isVpnIface("xfrm0"))
        assertEquals("utun3", true, IfaceLists.isVpnIface("utun3"))
        assertEquals("l2tp0", true, IfaceLists.isVpnIface("l2tp0"))
        assertEquals("gre0", true, IfaceLists.isVpnIface("gre0"))
        assertEquals("TUN0", true, IfaceLists.isVpnIface("TUN0"))
        assertEquals("Wg99", true, IfaceLists.isVpnIface("Wg99"))
        assertEquals("MyVPN", true, IfaceLists.isVpnIface("MyVPN"))
        assertEquals("custom_VPN_42", true, IfaceLists.isVpnIface("custom_VPN_42"))
        assertEquals("myvpn0", true, IfaceLists.isVpnIface("myvpn0"))
        assertEquals("vpn", true, IfaceLists.isVpnIface("vpn"))
        assertEquals("xvpn1", true, IfaceLists.isVpnIface("xvpn1"))
        assertEquals("lo", false, IfaceLists.isVpnIface("lo"))
        assertEquals("wlan0", false, IfaceLists.isVpnIface("wlan0"))
        assertEquals("wlan", false, IfaceLists.isVpnIface("wlan"))
        assertEquals("rmnet0", false, IfaceLists.isVpnIface("rmnet0"))
        assertEquals("rmnet_data0", false, IfaceLists.isVpnIface("rmnet_data0"))
        assertEquals("rmnet_ipa0", false, IfaceLists.isVpnIface("rmnet_ipa0"))
        assertEquals("eth0", false, IfaceLists.isVpnIface("eth0"))
        assertEquals("ccmni0", false, IfaceLists.isVpnIface("ccmni0"))
        assertEquals("seth_lte8", false, IfaceLists.isVpnIface("seth_lte8"))
        assertEquals("dummy0", false, IfaceLists.isVpnIface("dummy0"))
        assertEquals("bnep0", false, IfaceLists.isVpnIface("bnep0"))
        assertEquals("rndis0", false, IfaceLists.isVpnIface("rndis0"))
        assertEquals("if33", false, IfaceLists.isVpnIface("if33"))
        assertEquals("", false, IfaceLists.isVpnIface(""))
        assertEquals("tunl", true, IfaceLists.isVpnIface("tunl"))
        assertEquals("atun0", false, IfaceLists.isVpnIface("atun0"))
        assertEquals("VPN", true, IfaceLists.isVpnIface("VPN"))
    }
}
