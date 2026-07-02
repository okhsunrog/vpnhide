package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.generated.HookIds

/**
 * Compatibility holder for the Kotlin wire formatter tests and for launching
 * native activators. The app no longer fans out wire snapshots itself: it writes
 * canonical JSON, then runs the installed native module's activator.
 */
internal object ConfigChannels {
    // The hookmask written for legacy native config snapshots. The canonical
    // JSON stores per-app native hook lists; installed native activators fold
    // those into this protocol mask.
    private val FULL_MASK = HookIds.KERNEL_HOOK_MASK.toLong()

    /** A `vpnhide 1 config` snapshot for [uids] with [debug] folded in (§4.3). */
    fun config(
        debug: Boolean,
        uids: Collection<Int>,
    ): String =
        Protocol.formatConfig(
            debug,
            uids.toSortedSet().map { Protocol.Target(it.toLong(), FULL_MASK) },
        )

    /** Shell part running exactly one native activator by backend priority. */
    fun nativeActivatorCommand(): String =
        "if [ -x $KMOD_ACTIVATOR ] && [ ! -f $KMOD_MODULE_DIR/disable ]; then $KMOD_ACTIVATOR; " +
            "elif [ -x $KPM_ACTIVATOR ] && [ ! -f $KPM_MODULE_DIR/disable ]; then $KPM_ACTIVATOR; " +
            "elif [ -x $ZYGISK_ACTIVATOR ] && [ ! -f $ZYGISK_MODULE_DIR/disable ]; then $ZYGISK_ACTIVATOR; " +
            "else true; fi"

    /** Shell part running the optional ports activator when its module is enabled. */
    fun portsActivatorCommand(): String =
        "if [ -x $PORTS_ACTIVATOR ] && [ ! -f $PORTS_MODULE_DIR/disable ]; then $PORTS_ACTIVATOR 2>&1; else true; fi"

    /** Shell part running exactly one native activator by backend priority. */
    fun nativeWriteParts(): List<String> =
        listOf(
            nativeActivatorCommand(),
        )

    /**
     * Re-emit the runtime config for the current canonical config. Package→UID
     * resolution and wire formatting live in the activator, not in the app.
     */
    fun reconcileCommand(): String = nativeActivatorCommand()
}

/**
 * Run the startup runtime-config reconcile over root: derive the targets from
 * [rootSnapshot], fold in the persisted debug flag, and (re-)write the
 * `vpnhide 1 config` snapshot to every live channel. Blocking — call from a
 * background dispatcher. Best-effort: a non-zero exit is logged, not fatal.
 */
internal fun runRuntimeConfigReconcile(
    context: Context,
    rootSnapshot: RootSnapshot,
) {
    val snapshot = parseTargetsSnapshot(rootSnapshot)
    val persistedDebug = isEnabledInPrefs(context)
    val parts = mutableListOf<String>()
    runtimeReconcileCanonicalConfig(snapshot, persistedDebug)?.let { canonical ->
        parts += buildCanonicalConfigWriteCommand(canonical)
    }
    parts += ConfigChannels.reconcileCommand()
    val cmd = parts.joinToString(" ; ")
    val (exit, _) = suExec(cmd)
    if (exit != 0) VpnHideLog.w("VpnHide-Startup", "runtime config reconcile failed (exit=$exit)")
}

internal fun runtimeReconcileCanonicalConfig(
    snapshot: TargetsSnapshot,
    persistedDebug: Boolean,
): CanonicalConfig? {
    val existing = snapshot.canonicalConfig
    return when {
        existing == null -> {
            buildCanonicalConfig(
                debug = persistedDebug,
                javaPkgs = snapshot.lsposedTargets,
                nativePkgs = snapshot.nativeTargets,
                hiddenPkgs = snapshot.hiddenPkgs,
                observerPkgs = snapshot.observerNames,
                portsPkgs = snapshot.portsObservers,
            )
        }

        existing.debug != persistedDebug -> {
            existing.copy(debug = persistedDebug)
        }

        else -> {
            null
        }
    }
}

/**
 * Re-materialize `settings.autoHiddenPackages` for the on-disk [config] against
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
internal fun reconcileAutoHiddenPackages(
    context: Context,
    config: CanonicalConfig,
    signals: Collection<AppAutoHideSignal>,
): Boolean {
    val selfPkg = context.packageName
    if (!autoHiddenPackagesNeedReconcile(config, selfPkg, signals)) return false
    val next = applyAutoHiddenPackages(config, selfPkg, signals)
    val cmd =
        listOf(
            buildCanonicalConfigWriteCommand(next),
            ConfigChannels.reconcileCommand(),
        ).joinToString(" ; ")
    val (exit, output) = suExec(cmd)
    if (exit != 0) {
        VpnHideLog.w("VpnHide-Startup", "auto-hide reconcile failed (exit=$exit): ${output.trim()}")
        return false
    }
    RootSnapshotCache.invalidate()
    TargetsCache.invalidate()
    DashboardCache.invalidate()
    StatisticsCache.invalidate()
    VpnHideLog.i(
        "VpnHide-Startup",
        "auto-hide reconcile: ${next.settings.autoHiddenPackages.size} auto-hidden package(s)",
    )
    return true
}
