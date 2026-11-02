package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.settings.FilesystemHidingState
import dev.okhsunrog.vpnhide.settings.FilesystemHidingStatus

internal const val CORE_PACKAGE_ARTIFACT = "vpnhide-core.zip"

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

    data class CoreVersionMismatch(
        val moduleVersion: String,
        val appVersion: String,
        /** Offered only when the installed Core package is the older side. */
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
}

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
    /** Not read by any guard; carried so the orchestrator need not re-derive it. */
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
)

internal data class DashboardFacts(
    val modules: ModuleFacts,
    val lsposed: LsposedFacts,
    val targets: TargetCounts,
    val environment: EnvironmentFacts,
    val protection: ProtectionFacts,
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
        addAll(lsposedIssues(facts))
        addAll(moduleVersionIssues(facts))
        addAll(targetIssues(facts))
        addAll(environmentIssues(facts))
        addAll(moduleProblemIssues(facts))
        addAll(protectionIssues(facts))
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
    facts.modules.mismatches.filter { it.kind == FlashableModuleKind.Core }.map { mismatch ->
        // Offer the newer module for download only when the installed module is the
        // older side — a module newer than the app means the app is what's behind,
        // and re-flashing the module would not fix that.
        val moduleOlder =
            (compareSemver(baseVersion(mismatch.moduleVersion), baseVersion(mismatch.appVersion)) ?: 0) < 0
        DashboardIssue.CoreVersionMismatch(
            moduleVersion = mismatch.moduleVersion,
            appVersion = mismatch.appVersion,
            downloadArtifact = if (moduleOlder) CORE_PACKAGE_ARTIFACT else null,
        )
    }

private fun targetIssues(facts: DashboardFacts): List<DashboardIssue> =
    buildList {
        // A fresh install with nothing selected is not broken — guide, don't alarm.
        if (facts.targets.lsposed + facts.targets.native == 0) add(DashboardIssue.NoTargets)
        facts.environment.portsApply?.let { add(DashboardIssue.PortsRulesInactive(it.failureDetail)) }
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

        FilesystemHidingStatus.Unknown,
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
        val core = facts.modules.ports
        core.problem?.let { add(DashboardIssue.ModuleBroken(it)) }
        if (core.pendingReboot) add(DashboardIssue.ModuleNeedsReboot(FlashableModuleKind.Core))
    }

/**
 * The hook code lives in system_server and only swaps on reboot, so reinstalling
 * the APK — even on the same base version — leaves the old hooks running until
 * then. The comparison is therefore on the FULL version, dev suffix included:
 * that skew is the whole point of the notice. A developer who reinstalls all day
 * turns the notice off entirely (`ignoreVersionMismatch`) rather than getting a
 * blunter comparison.
 */
private fun protectionIssues(facts: DashboardFacts): List<DashboardIssue> {
    if (facts.developer.ignoreVersionMismatch) return emptyList()
    val running = (facts.lsposed.state as? LsposedState.Active)?.version ?: return emptyList()
    if (!versionsMismatchFull(running, facts.appVersion)) return emptyList()
    return listOf(
        DashboardIssue.LsposedVersionMismatch(
            runningVersion = running,
            appVersion = facts.appVersion,
            degraded = false,
        ),
    )
}
