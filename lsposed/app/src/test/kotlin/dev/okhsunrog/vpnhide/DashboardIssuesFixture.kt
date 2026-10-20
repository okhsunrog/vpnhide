package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
import dev.okhsunrog.vpnhide.settings.FilesystemHidingState
import dev.okhsunrog.vpnhide.settings.FilesystemHidingStatus

/** A healthy device for the dashboard guard tests, so each test states only its own deviation. */
internal abstract class DashboardIssuesFixture {
    protected val bootId = "boot-1"

    protected fun installed(
        version: String? = "1.2.5",
        active: Boolean = true,
    ) = ModuleState.Installed(version = version, active = active)

    protected fun moduleFacts(
        kmod: ModuleState = installed(),
        kpm: ModuleState = ModuleState.NotInstalled,
        zygisk: ModuleState = ModuleState.NotInstalled,
        ports: ModuleState = ModuleState.NotInstalled,
        builtin: ModuleState = ModuleState.NotInstalled,
        kmodProblem: ModuleProblem? = null,
        pendingReboot: Set<FlashableModuleKind> = emptySet(),
        kpmLoadStatus: KpmLoadStatus = kpmStatus(),
        mismatches: List<ModuleMismatch> = emptyList(),
        nativeBoot: NativeBootStatus? = null,
    ): ModuleFacts {
        val backends = NativeBackendStates(kmod = kmod, kpm = kpm, zygisk = zygisk, builtin = builtin)
        return ModuleFacts(
            kmod = ModuleFact(kmod, kmodProblem, FlashableModuleKind.Kmod in pendingReboot),
            builtin = ModuleFact(builtin, null, FlashableModuleKind.Builtin in pendingReboot),
            kpm = ModuleFact(kpm, null, FlashableModuleKind.Kpm in pendingReboot),
            zygisk = ModuleFact(zygisk, null, FlashableModuleKind.Zygisk in pendingReboot),
            ports = ModuleFact(ports, null, FlashableModuleKind.Core in pendingReboot),
            backends = backends,
            nativeBackend = displayNativeBackend(backends),
            kpmLoadStatus = kpmLoadStatus,
            kmodLoadStatus = null,
            currentBootId = bootId,
            mismatches = mismatches,
            nativeBoot = nativeBoot,
        )
    }

    protected fun boot(
        outcome: NativeBootOutcome,
        installed: List<NativeBackendId> = emptyList(),
        live: List<NativeBackendId> = emptyList(),
        staged: List<NativeBackendId> = emptyList(),
        zygiskTargets: ZygiskTargetsState = ZygiskTargetsState.NotInstalled,
        id: String = bootId,
    ) = NativeBootStatus(
        bootId = id,
        decidedAtMs = 1,
        outcome = outcome,
        installed = installed,
        live = live,
        staged = staged,
        kmod = null,
        zygiskTargets = zygiskTargets,
        ports = null,
    )

    protected fun kpmStatus(
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

    protected fun facts(
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
        developer: DeveloperFlags = DeveloperFlags(),
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
        developer = developer,
    )

    protected fun environment(
        selinuxPermissive: Boolean = false,
        selfProfileCount: Int = 1,
        debugLoggingOn: Boolean = false,
        filesystemHiding: FilesystemHidingState = FilesystemHidingState(FilesystemHidingStatus.Disabled),
        portsApply: PortsApplyProblem? = null,
        obsoleteModules: List<String> = emptyList(),
    ) = EnvironmentFacts(
        selinuxPermissive = selinuxPermissive,
        selfProfileCount = selfProfileCount,
        debugLoggingOn = debugLoggingOn,
        filesystemHiding = filesystemHiding,
        portsApply = portsApply,
        obsoleteModules = obsoleteModules,
    )

    protected fun protection(
        check: ProtectionCheck = ProtectionCheck.Checked(clean, clean),
        gap: PartialHookGap? = null,
    ) = ProtectionFacts(
        check = check,
        report = null,
        partialHookGap = gap,
        installedOptionalHooks = emptySet(),
    )

    protected fun recommendation(
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

    protected val clean = LayerStatus.Active(hidden = 6, leaks = 0)
    protected val leaking = LayerStatus.Active(hidden = 4, leaks = 2)

    protected inline fun <reified T : DashboardIssue> List<DashboardIssue>.has(): Boolean = any { it is T }
}
