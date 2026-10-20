package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

/** A 1.x companion module core replaced (Ports, Built-in) still installed after the upgrade. */
internal class ObsoleteModulesIssueTest : DashboardIssuesFixture() {
    @Test
    fun `the root snapshot section lists one module id per line`() {
        assertEquals(
            listOf("vpnhide_ports", "vpnhide_builtin"),
            parseObsoleteModules(mapOf(OBSOLETE_MODULES_SECTION to "vpnhide_ports\n vpnhide_builtin \n\n")),
        )
        assertEquals(emptyList<String>(), parseObsoleteModules(mapOf(OBSOLETE_MODULES_SECTION to "")))
        // An older core that does not publish the section: nothing to report.
        assertEquals(emptyList<String>(), parseObsoleteModules(emptyMap()))
    }

    @Test
    fun `an installed obsolete module is an error banner naming it`() {
        val issues =
            dashboardIssues(facts(environment = environment(obsoleteModules = listOf("vpnhide_ports"))))

        assertEquals(
            DashboardIssue.ObsoleteModulesInstalled(listOf("vpnhide_ports")),
            issues.filterIsInstance<DashboardIssue.ObsoleteModulesInstalled>().single(),
        )
        assertEquals(
            DashboardIssueReason("obsolete_modules_installed", mapOf("modules" to "vpnhide_ports")),
            issues.filterIsInstance<DashboardIssue.ObsoleteModulesInstalled>().single().toReason(),
        )
    }

    @Test
    fun `no obsolete module, no banner`() {
        assertFalse(dashboardIssues(facts()).has<DashboardIssue.ObsoleteModulesInstalled>())
    }
}
