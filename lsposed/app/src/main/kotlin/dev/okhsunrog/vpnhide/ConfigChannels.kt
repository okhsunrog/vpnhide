package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.picker.AppAutoHideSignal
import dev.okhsunrog.vpnhide.picker.applyAutoHiddenPackages

/**
 * Run the startup runtime-config reconcile: ask core to re-apply Native from its
 * durable config (`RepairActivation`). Blocking — call from a background
 * dispatcher. Best-effort: a failure is logged, not fatal.
 */
internal suspend fun runRuntimeConfigReconcile() {
    CanonicalConfigRepository.reconcile()
}

/**
 * Re-materialize `settings.autoHiddenPackages` for the fresh on-disk config against
 * fresh VpnService [signals], and persist it iff the auto-hidden set changed.
 * This keeps a newly-installed VPN app hidden from observers after a Hiding-tab
 * Refresh or a cold start, without the user having to open the picker and Save.
 *
 * Idempotent: [applyAutoHiddenPackages] only touches the auto-hidden set and the
 * hidden flags derived from it — every manual role (Java / Native / Apps / Ports
 * and manually-hidden packages) is preserved — so an unchanged set writes
 * nothing. Best-effort, blocking; call from a background dispatcher. Returns
 * true when it wrote (and re-activated) a new config.
 */
internal suspend fun reconcileAutoHiddenPackages(
    context: Context,
    signals: Collection<AppAutoHideSignal>,
): Boolean {
    val selfPkg = context.packageName
    val result =
        CanonicalConfigRepository.commit(
            CanonicalMutation(emptyList(), source = OperationSource.System, transform = { fresh ->
                applyAutoHiddenPackages(fresh, selfPkg, signals)
            }),
        )
    if (!result.succeeded) {
        VpnHideLog.w(
            LogTags.STARTUP,
            "auto-hide reconcile failed (exit=${result.exitCode}): ${result.output.trim()}",
        )
        return false
    }
    return result.operation?.phases?.get(ConfigPhase.Persist) == PhaseOutcome.Confirmed
}
