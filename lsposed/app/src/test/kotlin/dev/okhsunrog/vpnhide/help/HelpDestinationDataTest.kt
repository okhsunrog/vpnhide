package dev.okhsunrog.vpnhide.help

import org.junit.Assert.assertEquals
import org.junit.Test

class HelpDestinationDataTest {
    @Test
    fun contextualEntriesPreserveTheArticleAndAnchor() {
        assertEquals(HelpDestination(null, null), parseHelpDestination(null))
        assertEquals(HelpDestination("kpm-install", null), parseHelpDestination("kpm-install"))
        assertEquals(HelpDestination("kpm-install", null), parseHelpDestination("kpm-install#"))
        assertEquals(HelpDestination("kpm-install", "установка"), parseHelpDestination("kpm-install#установка"))
    }
}
