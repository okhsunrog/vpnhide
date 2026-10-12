package dev.okhsunrog.vpnhide

import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class DashboardIssueReasonTest {
    @Test
    fun `version reason and parameters survive localized wording changes`() {
        val issue = DashboardIssue.ModuleVersionMismatch(ModuleMismatch(FlashableModuleKind.Kmod, "1.2.0", "1.3.0"), null, null)
        val reason = issue.toReason()
        assertEquals("module_version_mismatch", reason.kind)
        assertEquals(mapOf("component" to "kmod", "moduleVersion" to "1.2.0", "appVersion" to "1.3.0"), reason.parameters)
        val english = DashboardMessage(DashboardMessageSeverity.WARNING, "Update module", reason = reason, presentationLanguage = "en")
        val russian = english.copy(text = "Обновите модуль", presentationLanguage = "ru")
        assertEquals(english.reason, russian.reason)
        assertTrue(Json.encodeToString(russian).contains("module_version_mismatch"))
    }

    @Test
    fun `hook and profile reasons retain structured counts`() {
        assertEquals(mapOf("count" to "3"), DashboardIssue.InstalledInMultipleProfiles(3).toReason().parameters)
        assertEquals("lsposed_config_unreadable", DashboardIssue.LsposedConfigUnreadable.toReason().kind)
        assertEquals("checks_failed", DashboardIssue.ChecksFailed.toReason().kind)
    }
}
