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
        assertEquals("v4-", false, IfaceLists.isNeverHide("v4-"))
        assertEquals("v4", false, IfaceLists.isNeverHide("v4"))
        assertEquals("tun0", false, IfaceLists.isNeverHide("tun0"))
        assertEquals("wg0", false, IfaceLists.isNeverHide("wg0"))
        assertEquals("wlan0", false, IfaceLists.isNeverHide("wlan0"))
        assertEquals("thread-wpan-extra", false, IfaceLists.isNeverHide("thread-wpan-extra"))
        assertEquals("if33", false, IfaceLists.isNeverHide("if33"))
        assertEquals("", false, IfaceLists.isNeverHide(""))
    }
}
