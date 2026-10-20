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

    data object RedundantKmodWithBuiltin : Native

    data object MultipleNativeActive : Native

    /**
     * Core found more than one native backend installed or live this boot, so it loaded
     * none and delivered no config. [backends] is every backend that took part.
     */
    data class NativeBackendsConflict(
        val backends: List<NativeBackendId>,
    ) : Native

    /** A module staged for the next reboot would leave more than one native backend. */
    data class NativeBackendsStagedConflict(
        val backends: List<NativeBackendId>,
    ) : Native

    /** KernelPatch cannot be queried and a VPN Hide KPM may be live: nothing else was loaded. */
    data class KpmUnverifiable(
        val detail: String,
    ) : Native

    /**
     * A live VPN Hide KPM (however it was loaded) that core cannot drive: older than
     * [minimum], an unrecognised version, or no valid status handshake.
     */
    data class KpmIncompatible(
        val found: String?,
        val minimum: String,
        val detail: String,
    ) : Native

    /**
     * The saved selection no longer fits the native backend (Repair or boot delivery):
     * nothing was sent and the backend keeps its previous targets.
     */
    data class NativeCapacityExceeded(
        val report: NativeCapacityRefusal,
    ) : Native

    /** Core's native boot step failed in a way no module card explains. */
    data class NativeBootFailed(
        val detail: String,
    ) : Native

    /** Zygisk is not the selected backend, but its old targets could not be cleared. */
    data class ZygiskRetireFailed(
        val detail: String,
    ) : Native

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
     * Integrity or load diagnosis shared with the module card via
     * `ModuleState.withBrokenReason`. Keep facts here; render the banner with
     * the current locale without reclassifying or refreshing root state.
     */
    data class ModuleBroken(
        val problem: ModuleProblem,
    ) : Module

    data class ModuleNeedsReboot(
        val kind: FlashableModuleKind,
    ) : Module

    // ── Target ──

    data object NoTargets : Target

    /** Ports rules are not in effect. [failureDetail] is set when this boot's apply failed. */
    data class PortsRulesInactive(
        val failureDetail: String?,
    ) : Target

    // ── Environment ──

    data class FilesystemHidingPending(
        val enabling: Boolean,
        val zygisk: Boolean,
    ) : Environment

    data object FilesystemHidingSetupError : Environment

    data object DebugLoggingOn : Environment

    data object AgentBridgeOn : Environment

    data object SelinuxPermissive : Environment

    /**
     * A 1.x companion module 2.0 replaced with core (`vpnhide_ports`, `vpnhide_builtin`)
     * is still installed. Its own boot scripts keep running beside core and fight it over
     * the iptables chain and the kernel control node, so it must be uninstalled.
     */
    data class ObsoleteModulesInstalled(
        val moduleIds: List<String>,
    ) : Environment

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
    val builtin: ModuleFact,
    val kpm: ModuleFact,
    val zygisk: ModuleFact,
    val ports: ModuleFact,
    val backends: NativeBackendStates,
    val nativeBackend: DisplayNativeBackend,
    val kpmLoadStatus: KpmLoadStatus,
    /** Not read by any guard; carried so the orchestrator need not re-derive it. */
    val kmodLoadStatus: KmodLoadStatus?,
    val currentBootId: String,
    val mismatches: List<ModuleMismatch>,
    val builtinKernelPresent: Boolean = false,
    /** Core's once-per-boot native decision; null when core did not report one. */
    val nativeBoot: NativeBootStatus? = null,
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
    val filesystemHiding: FilesystemHidingState,
    val portsApply: PortsApplyProblem?,
    /** Installed 1.x modules core replaced (root snapshot `obsolete_modules`). */
    val obsoleteModules: List<String> = emptyList(),
)

/**
 * The app-local switches on Settings → For developers that change what the
 * Dashboard says.
 *
 * They live here rather than in [EnvironmentFacts] because they are not derived
 * from root: the facts are cached until something invalidates them, so a flag
 * read in there only took effect after the next refresh. As part of the
 * projection they follow the toggle immediately — see [DeveloperFlagsCache].
 *
 * `debugLoggingOn` stays in [EnvironmentFacts] on purpose: it is the canonical
 * config's `debugSwitch`, owned by the config coordinator and genuinely read
 * off the device.
 */
internal data class DeveloperFlags(
    /** The debug localhost bridge is running; surfaced so it isn't left on unnoticed. */
    val agentBridgeOn: Boolean = false,
    /** Drop every module/hook-vs-app version notice; see AppSettings. */
    val ignoreVersionMismatch: Boolean = false,
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
    val developer: DeveloperFlags = DeveloperFlags(),
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
        // A KPM the user loaded by hand (or APatch autoloads) is a normal install:
        // detectKpmModule reports it as present, so it counts here like a module.
        if (!facts.modules.backends.anyInstalled) {
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
    if (facts.developer.ignoreVersionMismatch) {
        emptyList()
    } else {
        moduleVersionIssuesFor(facts)
    }

private fun moduleVersionIssuesFor(facts: DashboardFacts): List<DashboardIssue> =
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

internal fun downloadArtifactFor(
    kind: FlashableModuleKind,
    recommendedArtifact: String?,
): String? =
    when (kind) {
        FlashableModuleKind.Kmod -> recommendedArtifact

        // The in-tree backend has no downloadable zip that fixes a version skew:
        // the driver lives in the kernel image.
        FlashableModuleKind.Builtin -> null

        FlashableModuleKind.Kpm -> "vpnhide-kpm.zip"

        FlashableModuleKind.Zygisk -> "vpnhide-zygisk.zip"

        FlashableModuleKind.Core -> "vpnhide-core.zip"
    }

private fun targetIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        // A fresh install with nothing selected is not broken — guide, don't alarm.
        if (facts.targets.lsposed + facts.targets.native == 0) add(DashboardIssue.NoTargets)
        facts.environment.portsApply?.let { add(DashboardIssue.PortsRulesInactive(it.failureDetail)) }
    }

/** Working, but not the backend this kernel could be running. */
private fun nativeChoiceIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        addAll(betterBackendIssues(facts))
        addAll(multiNativeIssues(facts))
        addAll(nativeBootIssues(facts))
        if ((facts.modules.builtinKernelPresent || moduleActive(facts.modules.builtin.state)) &&
            facts.modules.kmod.state is ModuleState.Installed
        ) {
            add(DashboardIssue.RedundantKmodWithBuiltin)
        }
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
            modules.builtin.state is ModuleState.NotInstalled &&
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

        // The in-tree backend is never a recommendation: it can't be installed,
        // it's a property of how the kernel was built. Zygisk suggesting itself
        // is likewise a no-op.
        NativeBackendId.Builtin, NativeBackendId.Zygisk -> {
            emptyList()
        }
    }
}

private fun multiNativeIssues(facts: DashboardFacts): List<DashboardIssue> {
    val modules = facts.modules
    val boot = modules.nativeBoot?.takeIf { it.isFreshFor(modules.currentBootId) }
    // Core's own verdict comes first: with more than one candidate it loaded nothing, and
    // that names every backend involved, which the active-pair heuristics below cannot.
    (boot?.outcome as? NativeBootOutcome.Conflict)?.let {
        return listOf(DashboardIssue.NativeBackendsConflict(it.backends))
    }
    // Disabled or inactive modules may still have directories under
    // /data/adb/modules; they are not a freeze risk and must not raise the
    // .ko + KPM conflict banner.
    val severity =
        classifyMultiNative(
            kmodActive = moduleActive(modules.kmod.state),
            builtinActive = moduleActive(modules.builtin.state),
            kpmActive = moduleActive(modules.kpm.state),
            zygiskActive = moduleActive(modules.zygisk.state),
        )
    return buildList {
        when (severity) {
            MultiNativeSeverity.Error -> add(DashboardIssue.NativeConflictKernel)
            MultiNativeSeverity.Warning -> add(DashboardIssue.MultipleNativeActive)
            MultiNativeSeverity.None -> Unit
        }
        boot?.stagedConflict?.takeIf { it.isNotEmpty() }?.let { add(DashboardIssue.NativeBackendsStagedConflict(it)) }
    }
}

/**
 * Core's native boot failures that no module card diagnoses. KPM and kmod load failures
 * are left to [classifyKpmProblem] / [classifyKmodProblem], which own their wording.
 */
private fun nativeBootIssues(facts: DashboardFacts): List<DashboardIssue> {
    val boot = facts.modules.nativeBoot?.takeIf { it.isFreshFor(facts.modules.currentBootId) } ?: return emptyList()
    return buildList {
        when (val outcome = boot.outcome) {
            is NativeBootOutcome.KpmUnverifiable -> {
                add(DashboardIssue.KpmUnverifiable(outcome.detail))
            }

            is NativeBootOutcome.KpmIncompatible -> {
                add(DashboardIssue.KpmIncompatible(outcome.found, outcome.minimum, outcome.detail))
            }

            is NativeBootOutcome.CapacityExceeded -> {
                add(DashboardIssue.NativeCapacityExceeded(outcome.report))
            }

            is NativeBootOutcome.ForeignOwner -> {
                add(DashboardIssue.NativeBootFailed("foreign kernel backend id ${outcome.id}"))
            }

            is NativeBootOutcome.Unknown -> {
                add(DashboardIssue.NativeBootFailed(outcome.detail))
            }

            is NativeBootOutcome.DeliveryFailed -> {
                if (outcome.backend != NativeBackendId.Kpm) add(DashboardIssue.NativeBootFailed(outcome.detail))
            }

            is NativeBootOutcome.UnsupportedKernel -> {
                if (outcome.backend != NativeBackendId.Kpm) add(DashboardIssue.NativeBootFailed(outcome.detail))
            }

            is NativeBootOutcome.LoadFailed -> {
                if (outcome.backend == NativeBackendId.Builtin || outcome.backend == NativeBackendId.Zygisk) {
                    add(DashboardIssue.NativeBootFailed(outcome.stderr))
                }
            }

            is NativeBootOutcome.MissingArtifact -> {
                if (outcome.backend == NativeBackendId.Builtin || outcome.backend == NativeBackendId.Zygisk) {
                    add(DashboardIssue.NativeBootFailed("missing ${outcome.path}"))
                }
            }

            is NativeBootOutcome.Selected,
            is NativeBootOutcome.Conflict,
            is NativeBootOutcome.AwaitingAuthentication,
            NativeBootOutcome.NoBackend,
            -> {
                // Healthy, or explained by the conflict banner and the module cards.
            }
        }
        (boot.zygiskTargets as? ZygiskTargetsState.RetireFailed)?.let { add(DashboardIssue.ZygiskRetireFailed(it.detail)) }
    }
}

private fun environmentIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        val env = facts.environment
        if (env.obsoleteModules.isNotEmpty()) add(DashboardIssue.ObsoleteModulesInstalled(env.obsoleteModules))
        addAll(filesystemHidingIssues(env.filesystemHiding))
        // Only adb/root can read the verbose lines, so this is a neutral note
        // rather than a problem.
        if (env.debugLoggingOn) add(DashboardIssue.DebugLoggingOn)
        // A loopback HTTP server is an on-device fingerprint; note it so it isn't
        // left running unnoticed.
        if (facts.developer.agentBridgeOn) add(DashboardIssue.AgentBridgeOn)
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
                FlashableModuleKind.Builtin to modules.builtin,
                FlashableModuleKind.Kpm to modules.kpm,
                FlashableModuleKind.Zygisk to modules.zygisk,
                FlashableModuleKind.Core to modules.ports,
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
        // Only alarm when a measurement actually shows a leak behind a missing
        // hook. No report yet (blocked/failed/first boot) means nothing is known
        // to leak, so stay quiet rather than speculate — fail closed. The
        // structural "partial" fact still shows on the backend card; this banner
        // is the "you are detectable, act" layer, which needs evidence.
        protection.partialHookGap
            ?.takeIf { gap -> protection.report?.let { gap.costsAnyVector(it) } == true }
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
 * the APK — even on the same base version — leaves the old hooks running until
 * then. The comparison is therefore on the FULL version, dev suffix included:
 * that skew is the whole point of the notice. A developer who reinstalls all day
 * turns the notice off entirely (`ignoreVersionMismatch`) rather than getting a
 * blunter comparison.
 */
private fun versionMismatchIssues(facts: DashboardFacts): List<DashboardIssue> {
    if (facts.developer.ignoreVersionMismatch) return emptyList()
    val running = (facts.lsposed.state as? LsposedState.Active)?.version ?: return emptyList()
    if (!versionsMismatchFull(running, facts.appVersion)) return emptyList()
    return listOf(
        DashboardIssue.LsposedVersionMismatch(
            runningVersion = running,
            appVersion = facts.appVersion,
            degraded = !protectionFullyPassed(facts.protection.check),
        ),
    )
}
