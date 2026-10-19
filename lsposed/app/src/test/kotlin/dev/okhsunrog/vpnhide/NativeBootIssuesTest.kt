package dev.okhsunrog.vpnhide

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Dashboard banners driven by core's once-per-boot native decision ([NativeBootStatus]). */
internal class NativeBootIssuesTest : DashboardIssuesFixture() {
    @Test
    fun `core conflict names every backend and replaces the active-pair heuristics`() {
        val conflict =
            NativeBootOutcome.Conflict(
                installed = listOf(NativeBackendId.Zygisk, NativeBackendId.Kmod),
                live = listOf(NativeBackendId.Builtin),
                staged = emptyList(),
            )
        val issues =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            kmod = installed(active = false),
                            zygisk = installed(),
                            builtin = installed(),
                            nativeBoot = boot(conflict, installed = conflict.installed, live = conflict.live),
                        ),
                ),
            )

        assertTrue(
            issues.contains(
                DashboardIssue.NativeBackendsConflict(listOf(NativeBackendId.Kmod, NativeBackendId.Builtin, NativeBackendId.Zygisk)),
            ),
        )
        assertFalse(issues.has<DashboardIssue.MultipleNativeActive>())
        assertFalse(issues.has<DashboardIssue.NativeBackendsStagedConflict>())
    }

    @Test
    fun `a conflict from an earlier boot is not reported`() {
        val conflict = NativeBootOutcome.Conflict(listOf(NativeBackendId.Kmod, NativeBackendId.Kpm), emptyList(), emptyList())
        val issues = dashboardIssues(facts(modules = moduleFacts(nativeBoot = boot(conflict, id = "old-boot"))))

        assertFalse(issues.has<DashboardIssue.NativeBackendsConflict>())
    }

    @Test
    fun `a staged second backend warns before the reboot`() {
        val selected = NativeBootOutcome.Selected(NativeBackendId.Kmod, NativeSelectionMode.Loaded, false)
        val issues =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            nativeBoot = boot(selected, installed = listOf(NativeBackendId.Kmod), staged = listOf(NativeBackendId.Kpm)),
                        ),
                ),
            )

        assertTrue(issues.contains(DashboardIssue.NativeBackendsStagedConflict(listOf(NativeBackendId.Kmod, NativeBackendId.Kpm))))
    }

    @Test
    fun `a staged update of the selected backend is not a conflict`() {
        val selected = NativeBootOutcome.Selected(NativeBackendId.Kmod, NativeSelectionMode.Loaded, false)
        val issues =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            nativeBoot = boot(selected, installed = listOf(NativeBackendId.Kmod), staged = listOf(NativeBackendId.Kmod)),
                        ),
                ),
            )

        assertFalse(issues.has<DashboardIssue.NativeBackendsStagedConflict>())
    }

    @Test
    fun `unverifiable KernelPatch, foreign owners and a failed Zygisk retire are surfaced`() {
        fun issuesFor(status: NativeBootStatus) = dashboardIssues(facts(modules = moduleFacts(nativeBoot = status)))

        assertTrue(
            issuesFor(boot(NativeBootOutcome.KpmUnverifiable("auth"))).contains(DashboardIssue.KpmUnverifiable("auth")),
        )
        assertTrue(issuesFor(boot(NativeBootOutcome.ForeignOwner(0x99))).has<DashboardIssue.NativeBootFailed>())
        assertTrue(
            issuesFor(boot(NativeBootOutcome.Unknown("ctl unreadable"))).contains(DashboardIssue.NativeBootFailed("ctl unreadable")),
        )
        val retired =
            boot(
                NativeBootOutcome.Selected(NativeBackendId.Kmod, NativeSelectionMode.Loaded, false),
                zygiskTargets = ZygiskTargetsState.RetireFailed("read-only"),
            )
        assertTrue(issuesFor(retired).contains(DashboardIssue.ZygiskRetireFailed("read-only")))
    }

    @Test
    fun `module load failures are left to the module cards`() {
        val failed =
            NativeBootOutcome.LoadFailed(NativeBackendId.Kmod, 1, "Invalid argument", emptyList(), true, true, false)
        val issues = dashboardIssues(facts(modules = moduleFacts(nativeBoot = boot(failed))))

        assertFalse(issues.has<DashboardIssue.NativeBootFailed>())
    }

    @Test
    fun `awaiting authentication from core revives the superkey banner`() {
        val status = boot(NativeBootOutcome.AwaitingAuthentication("auth failed"), installed = listOf(NativeBackendId.Kpm))
        val issues =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            kmod = ModuleState.NotInstalled,
                            kpm = installed(active = false),
                            kpmLoadStatus = status.kpmLoadStatus(),
                            nativeBoot = status,
                        ),
                ),
            )

        assertTrue(issues.has<DashboardIssue.KpmAwaitingSuperkey>())
    }
}
