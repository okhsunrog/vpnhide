package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.settings.FilesystemHidingState
import dev.okhsunrog.vpnhide.settings.FilesystemHidingStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The dashboard's guard list, which used to be ~290 lines inline in
 * `loadDashboardState` and therefore unreachable from a test: every branch went
 * through `res.getString`, and this module has no Robolectric.
 *
 * These assert the decision only. Wording lives in `DashboardIssueRender.kt` and
 * is the half that still needs a device.
 */
internal class DashboardIssuesTest : DashboardIssuesFixture() {
    // ── Fixtures — a healthy device, so each test states only its own deviation ──

    @Test
    fun `a fully working setup produces no issues at all`() {
        assertEquals(emptyList<DashboardIssue>(), dashboardIssues(facts()))
    }

    // ── Native presence ──

    @Test
    fun `a hand-loaded KPM is a normal native backend, not an install error`() {
        val issues =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            kmod = ModuleState.NotInstalled,
                            kpm = ModuleState.Installed(version = null, active = true),
                        ),
                ),
            )
    }

    // ── LSPosed ──

    @Test
    fun `an active hook heartbeat suppresses on-disk config complaints`() {
        // The DB says the module is not enabled, but the hooks are demonstrably
        // running this boot — believe the runtime, not the file.
        val issues =
            dashboardIssues(
                facts(
                    lsposed = LsposedState.Active(version = "1.2.5", targetCount = 3),
                    config = LsposedConfig.Disabled,
                ),
            )

        assertFalse(issues.has<DashboardIssue.LsposedNotEnabled>())
    }

    @Test
    fun `an inactive module with a disabled config is reported`() {
        val issues =
            dashboardIssues(
                facts(lsposed = LsposedState.InstalledInactive("1.2.5"), config = LsposedConfig.Disabled),
            )

        assertTrue(issues.has<DashboardIssue.LsposedNotEnabled>())
    }

    @Test
    fun `not-configured is only worth saying when the framework is actually installed`() {
        val withFramework =
            dashboardIssues(
                facts(
                    lsposed = LsposedState.NotInstalled,
                    framework = LsposedFramework.Installed(disabled = false),
                    config = LsposedConfig.ModuleNotConfigured,
                ),
            )
        val withoutFramework =
            dashboardIssues(
                facts(
                    lsposed = LsposedState.NotInstalled,
                    framework = LsposedFramework.NotInstalled,
                    config = LsposedConfig.ModuleNotConfigured,
                ),
            )

        assertTrue(withFramework.has<DashboardIssue.LsposedNotEnabled>())
        // Without the framework, "not enabled" is noise on top of "not installed".
        assertFalse(withoutFramework.has<DashboardIssue.LsposedNotEnabled>())
        assertTrue(withoutFramework.has<DashboardIssue.LsposedNotInstalled>())
    }

    @Test
    fun `an unreadable config is distinct from a disabled one`() {
        val issues = dashboardIssues(facts(lsposed = LsposedState.InstalledInactive("1.2.5"), config = null))
        assertTrue(issues.has<DashboardIssue.LsposedConfigUnreadable>())
    }

    @Test
    fun `extra scope entries warn but a missing system scope errors`() {
        val issues =
            dashboardIssues(
                facts(
                    lsposed = LsposedState.InstalledInactive("1.2.5"),
                    config =
                        LsposedConfig.Enabled(
                            entries = listOf("system", "com.example.app"),
                            hasSystemFramework = false,
                            extraEntries = listOf("com.example.app"),
                        ),
                ),
            )

        assertTrue(issues.has<DashboardIssue.LsposedNoSystemScope>())
        assertEquals(
            listOf("com.example.app"),
            issues.filterIsInstance<DashboardIssue.LsposedExtraScope>().single().entries,
        )
    }

    @Test
    fun `a field rename subsumes the install failures it caused`() {
        val both =
            dashboardIssues(facts(brokenFields = "mNetworkCapabilities", installFailures = "3"))
        val failuresOnly = dashboardIssues(facts(installFailures = "3"))

        assertTrue(both.has<DashboardIssue.LsposedFieldRename>())
        assertFalse(both.has<DashboardIssue.LsposedInstallFailures>())
        assertTrue(failuresOnly.has<DashboardIssue.LsposedInstallFailures>())
    }

    // ── Module versions ──

    @Test
    fun `an older module is offered for download, a newer one is not`() {
        val older =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            mismatches = listOf(ModuleMismatch(FlashableModuleKind.Core, "1.2.0", "1.2.5")),
                        ),
                ),
            ).filterIsInstance<DashboardIssue.CoreVersionMismatch>().single()
        val newer =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            mismatches = listOf(ModuleMismatch(FlashableModuleKind.Core, "1.3.0", "1.2.5")),
                        ),
                ),
            ).filterIsInstance<DashboardIssue.CoreVersionMismatch>().single()

        assertEquals("vpnhide-core.zip", older.downloadArtifact)
        // The app is the lagging side here — re-flashing the module fixes nothing.
        assertEquals(null, newer.downloadArtifact)
    }

    @Test
    fun `legacy backend version metadata cannot offer a separate installation`() {
        val backends = listOf(FlashableModuleKind.Kmod, FlashableModuleKind.Builtin, FlashableModuleKind.Kpm, FlashableModuleKind.Zygisk)
        val issues =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            mismatches = backends.map { ModuleMismatch(it, "1.0.0", "2.0.0") },
                        ),
                ),
            )
        assertFalse(issues.has<DashboardIssue.CoreVersionMismatch>())
    }

    @Test
    fun `an unconfigured install is guided, not alarmed`() {
        val issues = dashboardIssues(facts(targets = TargetCounts(lsposed = 0, native = 0, ports = 0)))
        assertTrue(issues.has<DashboardIssue.NoTargets>())
    }

    @Test
    fun `ports with no observers is not an issue, core always provides it`() {
        val issues =
            dashboardIssues(
                facts(
                    modules = moduleFacts(ports = installed()),
                    targets = TargetCounts(lsposed = 3, native = 3, ports = 0),
                ),
            )

        assertTrue(issues.none { it is DashboardIssue.PortsRulesInactive })
    }

    @Test
    fun `a failed ports apply carries its detail through`() {
        val issues =
            dashboardIssues(facts(environment = environment(portsApply = PortsApplyProblem("xtables lock"))))

        assertEquals(
            "xtables lock",
            issues.filterIsInstance<DashboardIssue.PortsRulesInactive>().single().failureDetail,
        )
    }

    // ── Environment ──

    @Test
    fun `permissive selinux, debug logging, the agent bridge and extra profiles are each reported`() {
        val issues =
            dashboardIssues(
                facts(
                    environment =
                        environment(
                            selinuxPermissive = true,
                            selfProfileCount = 2,
                            debugLoggingOn = true,
                        ),
                    developer = DeveloperFlags(agentBridgeOn = true),
                ),
            )

        assertTrue(issues.has<DashboardIssue.SelinuxPermissive>())
        assertTrue(issues.has<DashboardIssue.DebugLoggingOn>())
        assertTrue(issues.has<DashboardIssue.AgentBridgeOn>())
        assertEquals(
            2,
            issues.filterIsInstance<DashboardIssue.InstalledInMultipleProfiles>().single().profileCount,
        )
    }

    @Test
    fun `a single profile is not worth mentioning`() {
        assertFalse(
            dashboardIssues(facts(environment = environment(selfProfileCount = 1)))
                .has<DashboardIssue.InstalledInMultipleProfiles>(),
        )
    }

    @Test
    fun `filesystem hiding reports only its transient and error states`() {
        fun issuesFor(status: FilesystemHidingStatus) =
            dashboardIssues(
                facts(environment = environment(filesystemHiding = FilesystemHidingState(status))),
            )

        assertTrue(issuesFor(FilesystemHidingStatus.PendingEnable).has<DashboardIssue.FilesystemHidingPending>())
        assertTrue(issuesFor(FilesystemHidingStatus.HookSetupError).has<DashboardIssue.FilesystemHidingSetupError>())
        // A settled feature — on, off, or unsupported — has nothing to say.
        assertEquals(emptyList<DashboardIssue>(), issuesFor(FilesystemHidingStatus.Active))
        assertEquals(emptyList<DashboardIssue>(), issuesFor(FilesystemHidingStatus.Disabled))
        assertEquals(emptyList<DashboardIssue>(), issuesFor(FilesystemHidingStatus.Unavailable))
    }

    @Test
    fun `a pending filesystem-hiding change records which direction it is going`() {
        fun pendingFor(status: FilesystemHidingStatus) =
            dashboardIssues(
                facts(environment = environment(filesystemHiding = FilesystemHidingState(status))),
            ).filterIsInstance<DashboardIssue.FilesystemHidingPending>().single()

        assertTrue(pendingFor(FilesystemHidingStatus.PendingEnable).enabling)
        assertFalse(pendingFor(FilesystemHidingStatus.PendingDisable).enabling)
    }

    // ── Module problems ──

    @Test
    fun `inactive hooks cannot produce a version mismatch`() {
        // Nothing is running, so there is no running version to disagree with.
        assertFalse(
            dashboardIssues(facts(lsposed = LsposedState.InstalledInactive("1.2.0"), appVersion = "1.2.5"))
                .has<DashboardIssue.LsposedVersionMismatch>(),
        )
    }

    @Test
    fun `a dev rebuild on the same base still counts as a running-hook mismatch`() {
        // The hook only swaps on reboot, so a reinstall off the same release is
        // exactly the skew worth reporting — the compare is on the full version.
        assertTrue(
            dashboardIssues(
                facts(
                    lsposed = LsposedState.Active(version = "1.2.5-3-gabc1234", targetCount = 3),
                    appVersion = "1.2.5",
                ),
            ).has<DashboardIssue.LsposedVersionMismatch>(),
        )
    }

    @Test
    fun `ignoreVersionMismatch drops every version notice, module and hook alike`() {
        fun issuesWith(ignore: Boolean) =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            mismatches =
                                listOf(
                                    ModuleMismatch(FlashableModuleKind.Zygisk, "1.2.5", "1.3.0"),
                                    ModuleMismatch(FlashableModuleKind.Core, "1.2.5", "1.3.0"),
                                ),
                        ),
                    lsposed = LsposedState.Active(version = "1.2.5-268-g96631d53", targetCount = 3),
                    appVersion = "1.3.0",
                    developer = DeveloperFlags(ignoreVersionMismatch = ignore),
                ),
            )

        // The base versions genuinely differ here (1.2.5 vs 1.3.0), which is why
        // comparing base versions only was never enough to quiet this down.
        assertTrue(issuesWith(ignore = false).has<DashboardIssue.CoreVersionMismatch>())
        assertTrue(issuesWith(ignore = false).has<DashboardIssue.LsposedVersionMismatch>())
        assertFalse(issuesWith(ignore = true).has<DashboardIssue.CoreVersionMismatch>())
        assertFalse(issuesWith(ignore = true).has<DashboardIssue.LsposedVersionMismatch>())
    }

    @Test
    fun `ignoreVersionMismatch keeps everything that is not a version notice`() {
        val issues =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            mismatches = listOf(ModuleMismatch(FlashableModuleKind.Zygisk, "1.2.5", "1.3.0")),
                            pendingReboot = setOf(FlashableModuleKind.Core),
                        ),
                    environment = environment(selinuxPermissive = true),
                    appVersion = "1.3.0",
                    developer = DeveloperFlags(ignoreVersionMismatch = true),
                ),
            )

        // A flashed-but-not-active module is a reboot instruction, not version
        // noise, so it survives; so does everything unrelated.
        assertTrue(issues.has<DashboardIssue.ModuleNeedsReboot>())
        assertTrue(issues.has<DashboardIssue.SelinuxPermissive>())
    }

    // ── Ordering ──
}
