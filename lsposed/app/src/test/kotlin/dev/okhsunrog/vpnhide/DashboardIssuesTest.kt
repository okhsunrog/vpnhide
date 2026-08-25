package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
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
class DashboardIssuesTest {
    // ── Fixtures — a healthy device, so each test states only its own deviation ──

    private val bootId = "boot-1"

    private fun installed(
        version: String? = "1.2.5",
        active: Boolean = true,
    ) = ModuleState.Installed(version = version, active = active)

    private fun moduleFacts(
        kmod: ModuleState = installed(),
        kpm: ModuleState = ModuleState.NotInstalled,
        zygisk: ModuleState = ModuleState.NotInstalled,
        ports: ModuleState = ModuleState.NotInstalled,
        kmodProblem: ModuleProblem? = null,
        pendingReboot: Set<FlashableModuleKind> = emptySet(),
        standaloneKpm: Boolean = false,
        kpmLoadStatus: KpmLoadStatus = kpmStatus(),
        mismatches: List<ModuleMismatch> = emptyList(),
    ): ModuleFacts {
        val backends = NativeBackendStates(kmod = kmod, kpm = kpm, zygisk = zygisk)
        return ModuleFacts(
            kmod = ModuleFact(kmod, kmodProblem, FlashableModuleKind.Kmod in pendingReboot),
            kpm = ModuleFact(kpm, null, FlashableModuleKind.Kpm in pendingReboot),
            zygisk = ModuleFact(zygisk, null, FlashableModuleKind.Zygisk in pendingReboot),
            ports = ModuleFact(ports, null, FlashableModuleKind.Ports in pendingReboot),
            backends = backends,
            nativeBackend = displayNativeBackend(backends),
            standaloneKpm = standaloneKpm,
            kpmLoadStatus = kpmLoadStatus,
            kmodLoadStatus = null,
            currentBootId = bootId,
            mismatches = mismatches,
        )
    }

    private fun kpmStatus(
        reason: KpmFailureReason = KpmFailureReason.Ok,
        loaded: Boolean? = true,
        boot: String? = bootId,
    ) = KpmLoadStatus(
        timestamp = null,
        bootId = boot,
        unameR = null,
        runtime = KpmRuntime.Apatch,
        loaded = loaded,
        filesystemHiding = null,
        reason = reason,
        detail = null,
    )

    private fun facts(
        modules: ModuleFacts = moduleFacts(),
        lsposed: LsposedState = LsposedState.Active(version = "1.2.5", targetCount = 3),
        framework: LsposedFramework = LsposedFramework.Installed(disabled = false),
        config: LsposedConfig? = null,
        brokenFields: String? = null,
        installFailures: String? = null,
        targets: TargetCounts = TargetCounts(lsposed = 3, native = 3, ports = 0),
        environment: EnvironmentFacts = environment(),
        protection: ProtectionFacts = protection(),
        kernelRecommendation: NativeInstallRecommendation? = null,
        appVersion: String = "1.2.5",
    ) = DashboardFacts(
        modules = modules,
        lsposed =
            LsposedFacts(
                state = lsposed,
                framework = framework,
                config = config,
                brokenFields = brokenFields,
                installFailures = installFailures,
                aospSdkLabel = "35",
            ),
        targets = targets,
        environment = environment,
        protection = protection,
        kernelRecommendation = kernelRecommendation,
        appVersion = appVersion,
    )

    private fun environment(
        selinuxPermissive: Boolean = false,
        selfProfileCount: Int = 1,
        debugLoggingOn: Boolean = false,
        agentBridgeOn: Boolean = false,
        suppressVersionWarnings: Boolean = false,
        filesystemHiding: FilesystemHidingState = FilesystemHidingState(FilesystemHidingStatus.Disabled),
        portsApply: PortsApplyProblem? = null,
    ) = EnvironmentFacts(
        selinuxPermissive = selinuxPermissive,
        selfProfileCount = selfProfileCount,
        debugLoggingOn = debugLoggingOn,
        agentBridgeOn = agentBridgeOn,
        suppressVersionWarnings = suppressVersionWarnings,
        filesystemHiding = filesystemHiding,
        portsApply = portsApply,
    )

    private fun protection(
        check: ProtectionCheck = ProtectionCheck.Checked(clean, clean),
        gap: PartialHookGap? = null,
    ) = ProtectionFacts(
        check = check,
        report = null,
        partialHookGap = gap,
        installedOptionalHooks = emptySet(),
    )

    private fun recommendation(
        backend: NativeBackendId,
        kpatchRuntime: Boolean = false,
    ) = NativeInstallRecommendation(
        androidVersion = "Android 15",
        kernelVersion = "6.1.0",
        kernelBranch = "android14",
        recommended = backend,
        recommendedArtifact = "vpnhide-kmod-android14-6.1.zip",
        recommendedGkiVariant = "android14-6.1",
        kpatchRuntimeAvailable = kpatchRuntime,
    )

    private companion object {
        val clean = LayerStatus.Active(hidden = 6, leaks = 0)
        val leaking = LayerStatus.Active(hidden = 4, leaks = 2)
    }

    private inline fun <reified T : DashboardIssue> List<DashboardIssue>.has(): Boolean = any { it is T }

    // ── A healthy device says nothing ──

    @Test
    fun `a fully working setup produces no issues at all`() {
        assertEquals(emptyList<DashboardIssue>(), dashboardIssues(facts()))
    }

    // ── Native presence ──

    @Test
    fun `no native backend installed is reported`() {
        val issues = dashboardIssues(facts(modules = moduleFacts(kmod = ModuleState.NotInstalled)))
        assertTrue(issues.has<DashboardIssue.NoNativeBackend>())
    }

    @Test
    fun `a standalone KPM zip replaces the missing-native error rather than joining it`() {
        val issues =
            dashboardIssues(
                facts(modules = moduleFacts(kmod = ModuleState.NotInstalled, standaloneKpm = true)),
            )

        assertTrue(issues.has<DashboardIssue.KpmStandaloneInstall>())
        // Both would be telling the user to install a native backend; the specific
        // diagnosis wins.
        assertFalse(issues.has<DashboardIssue.NoNativeBackend>())
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
                            mismatches = listOf(ModuleMismatch(FlashableModuleKind.Zygisk, "1.2.0", "1.2.5")),
                        ),
                ),
            ).filterIsInstance<DashboardIssue.ModuleVersionMismatch>().single()
        val newer =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            mismatches = listOf(ModuleMismatch(FlashableModuleKind.Zygisk, "1.3.0", "1.2.5")),
                        ),
                ),
            ).filterIsInstance<DashboardIssue.ModuleVersionMismatch>().single()

        assertEquals("vpnhide-zygisk.zip", older.downloadArtifact)
        // The app is the lagging side here — re-flashing the module fixes nothing.
        assertEquals(null, newer.downloadArtifact)
    }

    @Test
    fun `an outdated kmod is offered the kernel's own variant`() {
        val issue =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            mismatches = listOf(ModuleMismatch(FlashableModuleKind.Kmod, "1.2.0", "1.2.5")),
                        ),
                    kernelRecommendation = recommendation(NativeBackendId.Kmod),
                ),
            ).filterIsInstance<DashboardIssue.ModuleVersionMismatch>().single()

        assertEquals("vpnhide-kmod-android14-6.1.zip", issue.downloadArtifact)
    }

    // ── Backend choice ──

    @Test
    fun `a zygisk-only install is nudged towards kmod`() {
        val issues =
            dashboardIssues(
                facts(
                    modules = moduleFacts(kmod = ModuleState.NotInstalled, zygisk = installed()),
                    kernelRecommendation = recommendation(NativeBackendId.Kmod),
                ),
            )

        assertEquals(
            SuggestedBackend.Kmod,
            issues.filterIsInstance<DashboardIssue.BetterBackendAvailable>().single().backend,
        )
    }

    @Test
    fun `KPM is only suggested when a KPatch runtime is already there`() {
        fun issuesWith(kpatchRuntime: Boolean) =
            dashboardIssues(
                facts(
                    modules = moduleFacts(kmod = ModuleState.NotInstalled, zygisk = installed()),
                    kernelRecommendation = recommendation(NativeBackendId.Kpm, kpatchRuntime = kpatchRuntime),
                ),
            )

        assertTrue(issuesWith(kpatchRuntime = true).has<DashboardIssue.BetterBackendAvailable>())
        // Otherwise the nudge means "install two more things" — too pushy for a hint.
        assertFalse(issuesWith(kpatchRuntime = false).has<DashboardIssue.BetterBackendAvailable>())
    }

    @Test
    fun `two live kernel backends are an error, kernel plus zygisk only a warning`() {
        val twoKernel =
            dashboardIssues(facts(modules = moduleFacts(kmod = installed(), kpm = installed())))
        val kernelAndZygisk =
            dashboardIssues(facts(modules = moduleFacts(kmod = installed(), zygisk = installed())))

        assertTrue(twoKernel.has<DashboardIssue.NativeConflictKernel>())
        assertTrue(kernelAndZygisk.has<DashboardIssue.MultipleNativeActive>())
    }

    @Test
    fun `an inactive second backend is not a conflict`() {
        // Disabled modules keep their /data/adb/modules directory; only live ones
        // can freeze the kernel.
        val issues =
            dashboardIssues(
                facts(modules = moduleFacts(kmod = installed(), kpm = installed(active = false))),
            )

        assertFalse(issues.has<DashboardIssue.NativeConflictKernel>())
        assertFalse(issues.has<DashboardIssue.MultipleNativeActive>())
    }

    @Test
    fun `KPM dormant for a missing superkey is surfaced`() {
        val issues =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            kpm = installed(active = false),
                            kpmLoadStatus = kpmStatus(reason = KpmFailureReason.AwaitingSuperkey, loaded = false),
                        ),
                ),
            )

        assertTrue(issues.has<DashboardIssue.KpmAwaitingSuperkey>())
    }

    // ── Targets and ports ──

    @Test
    fun `an unconfigured install is guided, not alarmed`() {
        val issues = dashboardIssues(facts(targets = TargetCounts(lsposed = 0, native = 0, ports = 0)))
        assertTrue(issues.has<DashboardIssue.NoTargets>())
    }

    @Test
    fun `ports installed with no observers is called out`() {
        val issues =
            dashboardIssues(
                facts(
                    modules = moduleFacts(ports = installed()),
                    targets = TargetCounts(lsposed = 3, native = 3, ports = 0),
                ),
            )

        assertTrue(issues.has<DashboardIssue.PortsNoObservers>())
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
                            agentBridgeOn = true,
                        ),
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
    fun `a module problem and a pending reboot are mutually exclusive per module`() {
        val problem = ModuleProblem(ModuleBrokenReason.WrongVariant, "broken", downloadArtifact = "x.zip")
        val issues =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            kmodProblem = problem,
                            pendingReboot = setOf(FlashableModuleKind.Ports),
                            ports = installed(),
                        ),
                ),
            )

        assertEquals(problem, issues.filterIsInstance<DashboardIssue.ModuleBroken>().single().problem)
        assertEquals(
            FlashableModuleKind.Ports,
            issues.filterIsInstance<DashboardIssue.ModuleNeedsReboot>().single().kind,
        )
    }

    // ── Protection ──

    @Test
    fun `a leaking owned vector is surfaced`() {
        val issues = dashboardIssues(facts(protection = protection(ProtectionCheck.Checked(leaking, clean))))
        assertTrue(issues.has<DashboardIssue.ChecksFailed>())
    }

    @Test
    fun `a blocked or failed run makes no leak claim`() {
        assertFalse(
            dashboardIssues(facts(protection = protection(ProtectionCheck.Failed)))
                .has<DashboardIssue.ChecksFailed>(),
        )
    }

    @Test
    fun `a partial hook gap with no measured report is reported anyway`() {
        // No report means the run could not measure; err towards telling the user.
        val issues =
            dashboardIssues(
                facts(protection = protection(gap = PartialHookGap(installed = 7, expected = 9, missing = emptyList()))),
            )

        assertTrue(issues.has<DashboardIssue.PartialHooks>())
    }

    @Test
    fun `a version mismatch is informational while everything passes and a warning once it does not`() {
        fun mismatchFor(check: ProtectionCheck) =
            dashboardIssues(
                facts(
                    lsposed = LsposedState.Active(version = "1.2.0", targetCount = 3),
                    appVersion = "1.2.5",
                    protection = protection(check),
                ),
            ).filterIsInstance<DashboardIssue.LsposedVersionMismatch>().single()

        assertFalse(mismatchFor(ProtectionCheck.Checked(clean, clean)).degraded)
        assertTrue(mismatchFor(ProtectionCheck.Checked(leaking, clean)).degraded)
    }

    @Test
    fun `inactive hooks cannot produce a version mismatch`() {
        // Nothing is running, so there is no running version to disagree with.
        assertFalse(
            dashboardIssues(facts(lsposed = LsposedState.InstalledInactive("1.2.0"), appVersion = "1.2.5"))
                .has<DashboardIssue.LsposedVersionMismatch>(),
        )
    }

    @Test
    fun `suppressVersionWarnings compares base versions only`() {
        fun mismatchesWith(suppress: Boolean) =
            dashboardIssues(
                facts(
                    lsposed = LsposedState.Active(version = "1.2.5-3-gabc1234", targetCount = 3),
                    appVersion = "1.2.5",
                    environment = environment(suppressVersionWarnings = suppress),
                ),
            ).has<DashboardIssue.LsposedVersionMismatch>()

        // A dev rebuild off the same base: the full compare notices, the base one does not.
        assertTrue(mismatchesWith(suppress = false))
        assertFalse(mismatchesWith(suppress = true))
    }

    // ── Ordering ──

    @Test
    fun `issues come out in the order the dashboard shows them`() {
        // The screen groups by severity but keeps emission order inside each group,
        // so this order decides which error a user reads first. It used to be
        // implied by the physical layout of one 290-line block.
        val issues =
            dashboardIssues(
                facts(
                    modules =
                        moduleFacts(
                            kmod = ModuleState.NotInstalled,
                            zygisk = installed(),
                            ports = installed(),
                            mismatches = listOf(ModuleMismatch(FlashableModuleKind.Zygisk, "1.2.0", "1.2.5")),
                            pendingReboot = setOf(FlashableModuleKind.Ports),
                        ),
                    lsposed = LsposedState.InstalledInactive("1.2.5"),
                    framework = LsposedFramework.NotInstalled,
                    config = LsposedConfig.ModuleNotConfigured,
                    targets = TargetCounts(lsposed = 0, native = 0, ports = 0),
                    environment = environment(selinuxPermissive = true, debugLoggingOn = true),
                    protection = protection(ProtectionCheck.Checked(leaking, clean)),
                    kernelRecommendation = recommendation(NativeBackendId.Kmod),
                ),
            )

        assertEquals(
            listOf(
                // No NoNativeBackend: zygisk is installed, so the native layer exists.
                "LsposedNotInstalled",
                "ModuleVersionMismatch",
                "NoTargets",
                "PortsNoObservers",
                "BetterBackendAvailable",
                "DebugLoggingOn",
                "SelinuxPermissive",
                "ModuleNeedsReboot",
                "ChecksFailed",
            ),
            issues.map { it::class.simpleName },
        )
    }
}
