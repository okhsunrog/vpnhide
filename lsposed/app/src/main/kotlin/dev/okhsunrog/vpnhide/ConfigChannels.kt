package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.picker.AppAutoHideSignal
import dev.okhsunrog.vpnhide.picker.applyAutoHiddenPackages

/**
 * Launches the native activators. The app does not fan out wire snapshots
 * itself: it writes canonical JSON, then runs the installed native module's
 * activator, which derives that backend's wire from the JSON.
 */
internal object ConfigChannels {
    /** The current mutation helper delegates selection to the shared native activation layer. */
    fun nativeActivatorCommand(): String = "\"${'$'}{VPNHIDE_MUTATION_HELPER:?mutation helper not supplied}\" activate native"

    /** Shell part running the optional ports activator when its module is enabled. */
    fun portsActivatorCommand(): String =
        "{ ${activatorShellHelper()}; " +
            "if [ -f $PORTS_MODULE_DIR/module.prop ] && [ ! -f $PORTS_MODULE_DIR/disable ]; then " +
            "run_activator $PORTS_ACTIVATOR Ports; else true; fi; }"

    /**
     * Re-emit the runtime config for the current canonical config. Package→UID
     * resolution and wire formatting live in the activator, not in the app.
     */
    fun reconcileCommand(): String = nativeActivatorCommand()

    /** Shared shell helper: selecting a module and validating its bundle are separate steps. */
    fun activatorShellHelper(): String =
        "run_activator() { " +
            "ACTIVATOR_PATH=\"${'$'}1\"; MODULE_LABEL=\"${'$'}2\"; " +
            "if [ -x \"${'$'}ACTIVATOR_PATH\" ]; then \"${'$'}ACTIVATOR_PATH\" 2>&1; " +
            "else echo \"vpnhide: ${'$'}MODULE_LABEL activator missing or not executable at " +
            "${'$'}ACTIVATOR_PATH\" >&2; return 1; fi; }"
}

/**
 * Run the startup runtime-config reconcile. Canonical JSON already contains
 * debug, so reconcile only needs to re-run activators to pick up the current
 * file state. Blocking — call from a background dispatcher. Best-effort: a
 * non-zero exit is logged, not fatal.
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
