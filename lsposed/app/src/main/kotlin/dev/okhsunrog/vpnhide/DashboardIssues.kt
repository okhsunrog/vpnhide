package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.DiagnosticReport
import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.settings.FilesystemHidingState
import dev.okhsunrog.vpnhide.settings.FilesystemHidingStatus

/**
 * What the dashboard has to say about this device, as data.
 *
 * The banner list used to be built inline in `loadDashboardState`: ~25 guards
 * interleaved with `res.getString` calls, which made the decision logic
 * unreachable from a unit test — every branch needed a real `Resources`, and
 * this module has no Robolectric. Splitting the decision (here, pure) from the
 * wording ([toMessage], thin) follows what the rest of the codebase already
 * does: `classifyKmodProblem`/`renderKmodProblem`,
 * `classifyKpmProblem`/`renderKpmProblem`. The filesystem-hiding banner used to
 * be the odd one out with its own decide-and-word function; it folds into the
 * same two halves here.
 *
 * A case carries exactly the data its wording needs, never a pre-formatted
 * string — otherwise the split would be cosmetic.
 *
 * Grouped into sub-interfaces so both the builder and the renderer stay a set
 * of small functions instead of one 28-branch `when` that would just trade a
 * `@Suppress("LongMethod")` for a `@Suppress("CyclomaticComplexMethod")`.
 */
internal sealed interface DashboardIssue {
    /** The native layer as a whole: nothing installed, the wrong thing installed, or two at once. */
    sealed interface Native : DashboardIssue

    /** LSPosed: the framework, our module's scope, and the hooks' own install health. */
    sealed interface Lsposed : DashboardIssue

    /** A specific flashable module: version drift, integrity, staged-for-reboot. */
    sealed interface Module : DashboardIssue

    /** Targets and the ports backend's applied rules. */
    sealed interface Target : DashboardIssue

    /** The device and the app's own settings, not any one module. */
    sealed interface Environment : DashboardIssue

    /** Derived from a completed check run, so these can only exist after one. */
    sealed interface Protection : DashboardIssue

    // ── Native ──

    /** The KPM zip was flashed as a plain module, so nothing loaded it. */
    data object KpmStandaloneInstall : Native

    data object NoNativeBackend : Native

    /**
     * A stealthier kernel backend fits this kernel but only Zygisk is installed.
     * [artifact] is the specific zip to flash, so the nudge can name it.
     *
     * Not [NativeBackendId]: suggesting Zygisk to a Zygisk user is not a state
     * this can be in, so it is not one the type allows.
     */
    data class BetterBackendAvailable(
        val backend: SuggestedBackend,
        val artifact: String,
    ) : Native

    /** Two kernel backends live at once — the .ko + KPM pair that freezes the device. */
    data object NativeConflictKernel : Native

    data object MultipleNativeActive : Native

    /** KPM stood down at boot because a .ko was already there. */
    data object NativeConflictDeferred : Native

    /** KPM is installed under APatch but dormant: no usable su token or saved SuperKey. */
    data object KpmAwaitingSuperkey : Native

    // ── LSPosed ──

    data object LsposedNotInstalled : Lsposed

    data object LsposedNeedsReboot : Lsposed

    data object LsposedConfigUnreadable : Lsposed

    data object LsposedNotEnabled : Lsposed

    data object LsposedNoSystemScope : Lsposed

    /** Scope entries beyond System Framework. They work; they are just noise. */
    data class LsposedExtraScope(
        val entries: List<String>,
    ) : Lsposed

    /**
     * The running AOSP renamed or retyped a field the hooks reflect on, so the
     * matching writeToParcel hook was skipped at install time.
     */
    data class LsposedFieldRename(
        val fields: String,
        val sdkLabel: String,
    ) : Lsposed

    data class LsposedInstallFailures(
        val detail: String,
    ) : Lsposed

    // ── Module ──

    data class ModuleVersionMismatch(
        val mismatch: ModuleMismatch,
        /** The kernel-specific zip to name for kmod; null for the other kinds. */
        val recommendedArtifact: String?,
        /** Offered for one-tap download only when the module is the older side. */
        val downloadArtifact: String?,
    ) : Module

    /**
     * Integrity or load diagnosis. Already rendered upstream: the same
     * [ModuleProblem] drives the module card's colour via
     * `ModuleState.withBrokenReason`, so the text exists before the issue list
     * is built and re-deriving it here would let card and banner drift apart.
     */
    data class ModuleBroken(
        val problem: ModuleProblem,
    ) : Module

    data class ModuleNeedsReboot(
        val kind: FlashableModuleKind,
    ) : Module

    // ── Target ──

    data object NoTargets : Target

    data object PortsNoObservers : Target

    /** Ports rules are not in effect. [failureDetail] is set when this boot's apply failed. */
    data class PortsRulesInactive(
        val failureDetail: String?,
    ) : Target

    // ── Environment ──

    data class FilesystemHidingPending(
        val enabling: Boolean,
        val zygisk: Boolean,
    ) : Environment

    data class FilesystemHidingBootError(
        val detail: String,
    ) : Environment

    data object FilesystemHidingSetupError : Environment

    data object DebugLoggingOn : Environment

    data object AgentBridgeOn : Environment

    data object SelinuxPermissive : Environment

    /** Installed in more than one user profile; a Save from the wrong one drops targets. */
    data class InstalledInMultipleProfiles(
        val profileCount: Int,
    ) : Environment

    // ── Protection ──

    data class PartialHooks(
        val installed: Int,
        val expected: Int,
        val missing: List<HookIds.Hook>,
    ) : Protection

    /**
     * The hooks running in system_server are a different build than this APK.
     * [degraded] downgrades it to a warning: with everything else passing this
     * is informational, but alongside a real failure it is a likely cause.
     */
    data class LsposedVersionMismatch(
        val runningVersion: String,
        val appVersion: String,
        val degraded: Boolean,
    ) : Protection

    /** A vector an active layer owns is leaking anyway. */
    data object ChecksFailed : Protection
}

/** The kernel backends worth nudging a Zygisk-only user towards. */
internal enum class SuggestedBackend { Kmod, Kpm }

// ── Facts ─────────────────────────────────────────────────────────────────
//
// The inputs the guards read, already derived from the root snapshot. Grouped
// rather than flat: one struct of 28 fields would trip detekt's constructor
// threshold, and the groups are the same ones the guards fall into anyway.

/** One flashable module, with everything the banners ask about it. */
internal data class ModuleFact(
    val state: ModuleState,
    /** Integrity/load diagnosis, or null when healthy or staged for reboot. */
    val problem: ModuleProblem?,
    val pendingReboot: Boolean,
)

internal data class ModuleFacts(
    val kmod: ModuleFact,
    val kpm: ModuleFact,
    val zygisk: ModuleFact,
    val ports: ModuleFact,
    val backends: NativeBackendStates,
    val nativeBackend: DisplayNativeBackend,
    /** The KPM zip is present as a plain module with no KernelPatch to load it. */
    val standaloneKpm: Boolean,
    val kpmLoadStatus: KpmLoadStatus,
    /** Not read by any guard; carried so the orchestrator need not re-derive it. */
    val kmodLoadStatus: KmodLoadStatus?,
    val currentBootId: String,
    val mismatches: List<ModuleMismatch>,
)

internal data class LsposedFacts(
    val state: LsposedState,
    val framework: LsposedFramework,
    /** Null means the on-disk config could not be read at all. */
    val config: LsposedConfig?,
    val brokenFields: String?,
    val installFailures: String?,
    val aospSdkLabel: String,
)

internal data class TargetCounts(
    val lsposed: Int,
    val native: Int,
    val ports: Int,
)

internal data class EnvironmentFacts(
    val selinuxPermissive: Boolean,
    val selfProfileCount: Int,
    val debugLoggingOn: Boolean,
    val agentBridgeOn: Boolean,
    /** Compare running-vs-installed by base version only; see SettingsRepository. */
    val suppressVersionWarnings: Boolean,
    val filesystemHiding: FilesystemHidingState,
    val portsApply: PortsApplyProblem?,
)

internal data class ProtectionFacts(
    val check: ProtectionCheck,
    /** The completed report, when there was one; null for a blocked or failed run. */
    val report: DiagnosticReport?,
    val partialHookGap: PartialHookGap?,
    val installedOptionalHooks: Set<HookIds.Hook>,
)

internal data class DashboardFacts(
    val modules: ModuleFacts,
    val lsposed: LsposedFacts,
    val targets: TargetCounts,
    val environment: EnvironmentFacts,
    val protection: ProtectionFacts,
    val kernelRecommendation: NativeInstallRecommendation?,
    val appVersion: String,
)

// ── The guard list ────────────────────────────────────────────────────────

/**
 * Every issue this device has, in the order the dashboard shows them.
 *
 * Order is load-bearing: the screen groups by severity but keeps emission
 * order inside each group, so the first error a user reads is decided here.
 * The eight calls below ARE that order — previously it was implied by the
 * physical layout of a 290-line block, where inserting a guard in the wrong
 * place silently reordered the banners.
 */
internal fun dashboardIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        addAll(nativePresenceIssues(facts))
        addAll(lsposedIssues(facts))
        addAll(moduleVersionIssues(facts))
        addAll(targetIssues(facts))
        addAll(nativeChoiceIssues(facts))
        addAll(environmentIssues(facts))
        addAll(moduleProblemIssues(facts))
        addAll(protectionIssues(facts))
    }

/** Is there a native layer at all, and was it installed in a way that can work. */
private fun nativePresenceIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        val modules = facts.modules
        if (modules.standaloneKpm) {
            add(DashboardIssue.KpmStandaloneInstall)
        } else if (!modules.backends.anyInstalled) {
            add(DashboardIssue.NoNativeBackend)
        }
    }

private fun lsposedIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        val lsposed = facts.lsposed
        val active = lsposed.state is LsposedState.Active
        if (lsposed.framework is LsposedFramework.NotInstalled && !active) {
            add(DashboardIssue.LsposedNotInstalled)
        }
        if (lsposed.state is LsposedState.NeedsReboot) {
            add(DashboardIssue.LsposedNeedsReboot)
        }
        // Config problems are only worth reporting when the hooks are not already
        // running: a live heartbeat proves the config works, whatever the on-disk
        // state looks like.
        if (!active) addAll(lsposedConfigIssues(lsposed))
        if (lsposed.brokenFields != null) {
            add(DashboardIssue.LsposedFieldRename(lsposed.brokenFields, lsposed.aospSdkLabel))
        }
        // A field rename explains the install failures it caused; reporting both
        // says the same thing twice.
        if (lsposed.installFailures != null && lsposed.brokenFields == null) {
            add(DashboardIssue.LsposedInstallFailures(lsposed.installFailures))
        }
    }

private fun lsposedConfigIssues(lsposed: LsposedFacts): List<DashboardIssue> =
    buildList {
        when (val config = lsposed.config) {
            null -> {
                add(DashboardIssue.LsposedConfigUnreadable)
            }

            LsposedConfig.ModuleNotConfigured -> {
                // With no framework installed this is already covered by
                // LsposedNotInstalled above.
                if (lsposed.framework is LsposedFramework.Installed) {
                    add(DashboardIssue.LsposedNotEnabled)
                }
            }

            LsposedConfig.Disabled -> {
                add(DashboardIssue.LsposedNotEnabled)
            }

            is LsposedConfig.Enabled -> {
                if (!config.hasSystemFramework) add(DashboardIssue.LsposedNoSystemScope)
                if (config.extraEntries.isNotEmpty()) {
                    add(DashboardIssue.LsposedExtraScope(config.extraEntries))
                }
            }
        }
    }

private fun moduleVersionIssues(facts: DashboardFacts): List<DashboardIssue> =
    facts.modules.mismatches.map { mismatch ->
        val recommendedArtifact =
            facts.kernelRecommendation
                ?.takeIf { mismatch.kind == FlashableModuleKind.Kmod && it.preferKmod }
                ?.recommendedArtifact
        // Offer the newer module for download only when the installed module is the
        // older side — a module newer than the app means the app is what's behind,
        // and re-flashing the module would not fix that.
        val moduleOlder =
            (compareSemver(baseVersion(mismatch.moduleVersion), baseVersion(mismatch.appVersion)) ?: 0) < 0
        DashboardIssue.ModuleVersionMismatch(
            mismatch = mismatch,
            recommendedArtifact = recommendedArtifact,
            downloadArtifact = if (moduleOlder) downloadArtifactFor(mismatch.kind, recommendedArtifact) else null,
        )
    }

private fun downloadArtifactFor(
    kind: FlashableModuleKind,
    recommendedArtifact: String?,
): String? =
    when (kind) {
        FlashableModuleKind.Kmod -> recommendedArtifact
        FlashableModuleKind.Kpm -> "vpnhide-kpm.zip"
        FlashableModuleKind.Zygisk -> "vpnhide-zygisk.zip"
        FlashableModuleKind.Ports -> "vpnhide-ports.zip"
    }

private fun targetIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        // A fresh install with nothing selected is not broken — guide, don't alarm.
        if (facts.targets.lsposed + facts.targets.native == 0) add(DashboardIssue.NoTargets)
        if (facts.modules.ports.state is ModuleState.Installed && facts.targets.ports == 0) {
            add(DashboardIssue.PortsNoObservers)
        }
        facts.environment.portsApply?.let { add(DashboardIssue.PortsRulesInactive(it.failureDetail)) }
    }

/** Working, but not the backend this kernel could be running. */
private fun nativeChoiceIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        addAll(betterBackendIssues(facts))
        addAll(multiNativeIssues(facts))
        if (facts.modules.kpm.state is ModuleState.Installed &&
            kpmAwaitingSuperkey(facts.modules.kpmLoadStatus, facts.modules.currentBootId)
        ) {
            add(DashboardIssue.KpmAwaitingSuperkey)
        }
    }

/**
 * Zygisk works but is detectable by anti-tamper apps when the Native role is on
 * for them, whereas kmod/KPM are invisible. Only nudge when the better backend
 * is installable right now: kmod always is, KPM only with a KPatch runtime
 * already present — otherwise replacing a working setup means installing two
 * more things, too pushy for a low-priority hint.
 */
private fun betterBackendIssues(facts: DashboardFacts): List<DashboardIssue> {
    val modules = facts.modules
    val onlyZygisk =
        modules.zygisk.state is ModuleState.Installed &&
            modules.kmod.state is ModuleState.NotInstalled &&
            modules.kpm.state is ModuleState.NotInstalled
    if (!onlyZygisk) return emptyList()
    val recommendation = facts.kernelRecommendation ?: return emptyList()
    return when (recommendation.recommended) {
        NativeBackendId.Kmod -> {
            listOf(DashboardIssue.BetterBackendAvailable(SuggestedBackend.Kmod, recommendation.recommendedArtifact))
        }

        NativeBackendId.Kpm -> {
            if (recommendation.kpatchRuntimeAvailable) {
                listOf(DashboardIssue.BetterBackendAvailable(SuggestedBackend.Kpm, recommendation.recommendedArtifact))
            } else {
                emptyList()
            }
        }

        NativeBackendId.Zygisk -> {
            emptyList()
        }
    }
}

private fun multiNativeIssues(facts: DashboardFacts): List<DashboardIssue> {
    val modules = facts.modules
    // Disabled or inactive modules may still have directories under
    // /data/adb/modules; they are not a freeze risk and must not raise the
    // .ko + KPM conflict banner.
    val severity =
        classifyMultiNative(
            kmodActive = moduleActive(modules.kmod.state),
            kpmActive = moduleActive(modules.kpm.state),
            zygiskActive = moduleActive(modules.zygisk.state),
        )
    return when (severity) {
        MultiNativeSeverity.Error -> {
            listOf(DashboardIssue.NativeConflictKernel)
        }

        MultiNativeSeverity.Warning -> {
            listOf(DashboardIssue.MultipleNativeActive)
        }

        MultiNativeSeverity.None -> {
            // The active-pair Error above is effectively unobservable — two live
            // kernel hookers freeze the device before this screen renders. KPM
            // standing down for a co-installed .ko is the state actually seen.
            if (kpmDeferredForConflict(modules.kpmLoadStatus, modules.currentBootId)) {
                listOf(DashboardIssue.NativeConflictDeferred)
            } else {
                emptyList()
            }
        }
    }
}

private fun environmentIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        val env = facts.environment
        addAll(filesystemHidingIssues(env.filesystemHiding))
        // Only adb/root can read the verbose lines, so this is a neutral note
        // rather than a problem.
        if (env.debugLoggingOn) add(DashboardIssue.DebugLoggingOn)
        // A loopback HTTP server is an on-device fingerprint; note it so it isn't
        // left running unnoticed.
        if (env.agentBridgeOn) add(DashboardIssue.AgentBridgeOn)
        // Permissive exposes the vectors we rely on SELinux to block (RTM_GETROUTE,
        // /proc/net/*, /sys/class/net) — see the coverage table in the README.
        if (env.selinuxPermissive) add(DashboardIssue.SelinuxPermissive)
        if (env.selfProfileCount > 1) {
            add(DashboardIssue.InstalledInMultipleProfiles(env.selfProfileCount))
        }
    }

private fun filesystemHidingIssues(state: FilesystemHidingState): List<DashboardIssue> {
    val zygisk = state.backend == NativeBackendId.Zygisk
    return when (state.status) {
        FilesystemHidingStatus.PendingEnable -> {
            listOf(DashboardIssue.FilesystemHidingPending(enabling = true, zygisk = zygisk))
        }

        FilesystemHidingStatus.PendingDisable -> {
            listOf(DashboardIssue.FilesystemHidingPending(enabling = false, zygisk = zygisk))
        }

        FilesystemHidingStatus.BootConfigError -> {
            listOf(DashboardIssue.FilesystemHidingBootError(state.errorDetail.orEmpty()))
        }

        FilesystemHidingStatus.HookSetupError -> {
            listOf(DashboardIssue.FilesystemHidingSetupError)
        }

        FilesystemHidingStatus.Unavailable,
        FilesystemHidingStatus.Disabled,
        FilesystemHidingStatus.Active,
        -> {
            emptyList()
        }
    }
}

/**
 * One banner per module, from the diagnosis already computed for its card, then
 * the staged-for-reboot warnings. Keeping both off the same [ModuleFact] is what
 * stops a banner's priority from drifting away from the card's colour.
 */
private fun moduleProblemIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        val modules = facts.modules
        val ordered =
            listOf(
                FlashableModuleKind.Kmod to modules.kmod,
                FlashableModuleKind.Kpm to modules.kpm,
                FlashableModuleKind.Zygisk to modules.zygisk,
                FlashableModuleKind.Ports to modules.ports,
            )
        ordered.forEach { (_, module) -> module.problem?.let { add(DashboardIssue.ModuleBroken(it)) } }
        ordered.forEach { (kind, module) ->
            if (module.pendingReboot) add(DashboardIssue.ModuleNeedsReboot(kind))
        }
    }

private fun protectionIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        val protection = facts.protection
        // A kernel backend that loaded but could not resolve every hook target.
        // Only worth saying when a missing hook costs a measurable vector: on
        // kernels that never had the symbol the surface is usually closed by
        // SELinux or a capability check anyway, and alarming there is noise.
        protection.partialHookGap
            ?.takeIf { gap -> protection.report?.let { gap.costsAnyVector(it) } != false }
            ?.let { add(DashboardIssue.PartialHooks(it.installed, it.expected, it.missing)) }
        addAll(versionMismatchIssues(facts))
        // A vector an active layer OWNS is leaking: the backend should have hidden
        // it and didn't, so the VPN is detectable AND the user can act (report the
        // device). Unowned leaks — vectors no active backend covers here — are
        // deliberately not surfaced: the backend is already doing all it can, and
        // alarming about a gap the user cannot close is noise. They still show,
        // neutrally, in the per-check breakdown.
        val checked = protection.check as? ProtectionCheck.Checked
        val nativeLeaks = (checked?.native as? LayerStatus.Active)?.leaks ?: 0
        val javaLeaks = (checked?.java as? LayerStatus.Active)?.leaks ?: 0
        if (nativeLeaks > 0 || javaLeaks > 0) add(DashboardIssue.ChecksFailed)
    }

/**
 * The hook code lives in system_server and only swaps on reboot, so reinstalling
 * the APK on the same base leaves the old hooks running until then. Developers
 * who reinstall constantly can flip `suppressVersionWarnings` to compare base
 * versions only; release users see no difference, release versions carrying no
 * dev suffix.
 */
private fun versionMismatchIssues(facts: DashboardFacts): List<DashboardIssue> {
    val running = (facts.lsposed.state as? LsposedState.Active)?.version ?: return emptyList()
    val mismatch =
        if (facts.environment.suppressVersionWarnings) {
            versionsMismatch(running, facts.appVersion)
        } else {
            versionsMismatchFull(running, facts.appVersion)
        }
    if (!mismatch) return emptyList()
    return listOf(
        DashboardIssue.LsposedVersionMismatch(
            runningVersion = running,
            appVersion = facts.appVersion,
            degraded = !protectionFullyPassed(facts.protection.check),
        ),
    )
}
