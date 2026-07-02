package dev.okhsunrog.vpnhide

import android.util.Log
import dev.okhsunrog.vpnhide.generated.IfaceLists
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

private const val TAG = "VpnHide"

// Legacy storage files retained as read-only migration inputs when canonical
// JSON is absent. New writes go to CANONICAL_CONFIG_FILE only.
// TODO(storage-migration): remove these shims after a few public releases once
// upgraded devices have had a chance to fold old files into canonical JSON.
internal const val KMOD_TARGETS = "/data/adb/vpnhide_kmod/targets.txt"
internal const val ZYGISK_TARGETS = "/data/adb/vpnhide_zygisk/targets.txt"
internal const val LSPOSED_TARGETS = "/data/adb/vpnhide_lsposed/targets.txt"

// The kmod's folded control+stats node (docs/protocol.md §OPEN-4): a write is a
// `vpnhide 1 config` snapshot, a read returns status+stats. Replaces the old
// /proc/vpnhide_targets (decimal UID list) + /proc/vpnhide_debug nodes.
internal const val PROC_CTL = "/proc/vpnhide_ctl"
internal const val SS_HIDDEN_PKGS_FILE = "/data/system/vpnhide_hidden_pkgs.txt"
internal const val SS_OBSERVER_UIDS_FILE = "/data/system/vpnhide_observer_uids.txt"
internal const val PORTS_OBSERVERS_FILE = "/data/adb/vpnhide_ports/observers.txt"
internal const val PORTS_LOAD_STATUS_FILE = "/data/adb/vpnhide_ports/load_status"
internal const val PORTS_LOAD_LOG_FILE = "/data/adb/vpnhide_ports/load_log"
internal const val PORTS_MODULE_DIR = "/data/adb/modules/vpnhide_ports"
internal const val PORTS_ACTIVATOR = "$PORTS_MODULE_DIR/activator"
internal const val KMOD_MODULE_DIR = "/data/adb/modules/vpnhide_kmod"
internal const val KMOD_LOAD_STATUS_FILE = "/data/adb/vpnhide_kmod/load_status"
internal const val KMOD_LOAD_DMESG_FILE = "/data/adb/vpnhide_kmod/load_dmesg"
internal const val ZYGISK_MODULE_DIR = "/data/adb/modules/vpnhide_zygisk"
internal const val APP_PACKAGE_NAME = "dev.okhsunrog.vpnhide"
internal const val ZYGISK_STATUS_FILE_NAME = "vpnhide_zygisk_active"
internal const val ZYGISK_STATUS_FILE = "/data/user/0/dev.okhsunrog.vpnhide/files/vpnhide_zygisk_active"

// KPM (KernelPatch Module) backend — the third native backend. The KPM has no
// /proc node; its runtime channel is the kpatch ctl0 supercall, so the app reads
// load_status for liveness instead of a proc marker.
internal const val KPM_TARGETS = "/data/adb/vpnhide_kpm/targets.txt"
internal const val KPM_MODULE_DIR = "/data/adb/modules/vpnhide_kpm"
internal const val KPM_LOAD_STATUS_FILE = "/data/adb/vpnhide_kpm/load_status"
internal const val KMOD_ACTIVATOR = "$KMOD_MODULE_DIR/activator"
internal const val KPM_ACTIVATOR = "$KPM_MODULE_DIR/activator"
internal const val ZYGISK_ACTIVATOR = "$ZYGISK_MODULE_DIR/activator"

/** Default cap on a single su invocation. Most root commands here finish
 *  in milliseconds; this only fires if the su binary is genuinely stuck
 *  (e.g. waiting on a GUI prompt that the user dismissed). */
internal const val SU_DEFAULT_TIMEOUT_SEC: Long = 10
private const val SELF_TARGETS_TIMEOUT_SEC: Long = SU_DEFAULT_TIMEOUT_SEC

/**
 * Returns exit code and stdout. Exit code -1 means the su binary
 * couldn't be executed at all (not installed, permission denied, or
 * still running after [timeoutSec] seconds — in which case it gets
 * destroyForcibly()'d so we don't leak the process).
 *
 * Both pipes are drained on dedicated threads — `readText()` directly
 * on `proc.inputStream` would block until EOF, so a hung child means
 * `waitFor(timeout)` is never even reached. The threads exit naturally
 * once the child (or destroyForcibly) closes its pipes.
 */
internal fun suExec(
    cmd: String,
    timeoutSec: Long = SU_DEFAULT_TIMEOUT_SEC,
): Pair<Int, String> =
    try {
        val proc = Runtime.getRuntime().exec(arrayOf("su", "-c", cmd))
        try {
            val stdoutHolder = AtomicReference("")
            val stdoutDrain =
                Thread {
                    runCatching { stdoutHolder.set(proc.inputStream.bufferedReader().readText()) }
                }
            val stderrDrain = Thread { runCatching { proc.errorStream.readBytes() } }
            stdoutDrain.start()
            stderrDrain.start()

            val finished = proc.waitFor(timeoutSec, TimeUnit.SECONDS)
            if (!finished) {
                Log.w(TAG, "su exec timed out after ${timeoutSec}s: ${commandPreview(cmd)}")
                proc.destroyForcibly()
            }
            // After destroyForcibly the pipes close and the drains exit;
            // a 1s join is plenty and bounds the worst case.
            stdoutDrain.join(1_000)
            stderrDrain.join(1_000)

            val exit = if (finished) proc.exitValue() else -1
            exit to stdoutHolder.get()
        } finally {
            proc.destroy()
        }
    } catch (e: Exception) {
        VpnHideLog.e(TAG, "su exec failed: ${e.message}")
        -1 to ""
    }

private fun commandPreview(cmd: String): String {
    val preview =
        cmd
            .lineSequence()
            .map { it.trimEnd() }
            .filter { it.isNotBlank() }
            .take(12)
            .joinToString("\n")
    return if (preview.length <= 800) preview else preview.take(800) + "..."
}

internal suspend fun suExecAsync(
    cmd: String,
    timeoutSec: Long = SU_DEFAULT_TIMEOUT_SEC,
): Pair<Int, String> = withContext(Dispatchers.IO) { suExec(cmd, timeoutSec) }

/**
 * Lines of a vpnhide config file: trimmed, with blank lines and `#`-comments
 * dropped. This is the on-disk format shared by every targets / observer /
 * hidden-packages file, so the parsing rule lives in one place.
 */
internal fun parseConfigLines(raw: String): List<String> =
    raw
        .lineSequence()
        .map { it.trim() }
        .filter { it.isNotEmpty() && !it.startsWith("#") }
        .toList()

/**
 * Parse `key=value` lines into a map. Values are kept verbatim (not trimmed) —
 * callers that need trimming do it themselves. Lines without an `=` are
 * dropped; the value keeps any further `=` (split limit 2). Single source for
 * every status-file / module.prop style blob this app reads back over root.
 */
internal fun parseKeyValueLines(raw: String): Map<String, String> =
    raw
        .lineSequence()
        .mapNotNull {
            val parts = it.split("=", limit = 2)
            if (parts.size == 2) parts[0] to parts[1] else null
        }.toMap()

private val PM_PACKAGE_UID_LINE = Regex("^package:(\\S+) uid:(\\S+)")

/**
 * Parse `pm list packages -U [--user all]` output (the form *without* `-f`,
 * so the token after `package:` is the package name, not an APK path) into
 * `package → uids`. Multi-profile packages report comma-separated UIDs
 * (`uid:10187,1010187`); repeated lines for the same package are unioned.
 * UIDs that aren't integers are dropped. The `-f` variant has a different
 * shape and is parsed separately in [AppListCache].
 */
internal fun parsePackageUidMap(raw: String): Map<String, List<Int>> {
    val out = LinkedHashMap<String, MutableList<Int>>()
    raw.lineSequence().forEach { line ->
        val m = PM_PACKAGE_UID_LINE.find(line) ?: return@forEach
        val uids = out.getOrPut(m.groupValues[1]) { mutableListOf() }
        m.groupValues[2].split(',').forEach { token ->
            token.trim().toIntOrNull()?.let(uids::add)
        }
    }
    return out
}

internal fun parseVpnIfaceStates(raw: String): List<Pair<String, String>> =
    raw
        .lineSequence()
        .mapNotNull { line ->
            val trimmed = line.trim()
            if (trimmed.isEmpty()) return@mapNotNull null

            val legacyParts = trimmed.split("=", limit = 2)
            if (legacyParts.size == 2) {
                return@mapNotNull legacyParts[0].trim() to legacyParts[1].trim()
            }

            val marker = "/operstate:"
            val markerIndex = trimmed.indexOf(marker)
            if (markerIndex <= 0) return@mapNotNull null
            val path = trimmed.substring(0, markerIndex)
            val iface = path.substringAfterLast('/').trim()
            val state = trimmed.substring(markerIndex + marker.length).trim()
            if (iface.isEmpty()) null else iface to state
        }.filter { (name, _) -> IfaceLists.isVpnIface(name) }
        .toList()

internal fun isVpnActiveFromStates(vpnIfaces: List<Pair<String, String>>): Boolean {
    if (vpnIfaces.isEmpty()) {
        VpnHideLog.d(TAG, "isVpnActive: no VPN interfaces found")
        return false
    }
    return vpnIfaces.any { (iface, state) ->
        val up = state == "unknown" || state == "up"
        VpnHideLog.d(TAG, "isVpnActive: $iface operstate=$state up=$up")
        up
    }
}

internal fun isVpnActiveFromSnapshot(raw: String): Boolean = isVpnActiveFromStates(parseVpnIfaceStates(raw))

/**
 * Single source of truth for the live shell probe. Startup paths should prefer
 * [isVpnActiveFromSnapshot] over this function when a root snapshot already
 * exists, so Dashboard and Diagnostics don't drift during one launch.
 */
internal fun isVpnActiveBlocking(): Boolean {
    val (exitCode, output) =
        suExec(
            "grep -H . /sys/class/net/*/operstate 2>/dev/null",
        )
    if (exitCode != 0) return false
    return isVpnActiveFromSnapshot(output)
}

/**
 * Why startup root-state preparation failed, so the UI can tell the user the
 * truth instead of always blaming root permissions. The distinction is known at
 * the point of failure (not parsed back out of a string).
 */
internal enum class SelfTargetFailureKind {
    /** The root command didn't run: denied, su prompt interrupted, non-zero exit. */
    RootUnavailable,

    /** Root worked, but the state probe returned incomplete data (a backend's
     *  section was missing — e.g. a broken/garbled KPM control reply). */
    IncompleteData,

    /** Root worked, but writing the updated config back failed. */
    ConfigWriteFailed,

    Unknown,
}

internal data class SelfTargetPreparation(
    val rootAvailable: Boolean,
    val selfNeedsRestart: Boolean,
    val currentBootId: String?,
    val pmPackages: String? = null,
    val error: String? = null,
    val failureKind: SelfTargetFailureKind = SelfTargetFailureKind.Unknown,
)

internal fun cleanupStaleZygiskStatus(
    context: android.content.Context,
    knownCurrentBootId: String? = null,
) {
    val statusFile = File(context.filesDir, ZYGISK_STATUS_FILE_NAME)
    if (!statusFile.isFile) return

    val props =
        try {
            parseKeyValueLines(statusFile.readText())
        } catch (e: Exception) {
            VpnHideLog.w(TAG, "cleanupStaleZygiskStatus: failed to read heartbeat: ${e.message}")
            emptyMap()
        }

    val heartbeatBootId = props["boot_id"]
    val currentBootId =
        knownCurrentBootId
            ?.trim()
            ?.takeIf { it.isNotEmpty() }
            ?: suExec("cat /proc/sys/kernel/random/boot_id 2>/dev/null").second.trim()
    val stale =
        heartbeatBootId.isNullOrBlank() ||
            heartbeatBootId != currentBootId

    if (stale) {
        if (statusFile.delete()) {
            VpnHideLog.i(
                TAG,
                "cleanupStaleZygiskStatus: deleted stale heartbeat " +
                    "(bootId=$heartbeatBootId currentBootId=$currentBootId)",
            )
        } else {
            VpnHideLog.w(TAG, "cleanupStaleZygiskStatus: failed to delete stale heartbeat")
        }
    }
}

/**
 * Ensure the VPN Hide app itself is in the canonical target config.
 * Returns true if self had to be added to any list (= hooks may not be
 * applied to the current process, restart needed for zygisk).
 * Called once at app startup; result is shared with all screens.
 */
internal fun ensureSelfInTargets(
    selfPkg: String,
    timeoutSec: Long = SELF_TARGETS_TIMEOUT_SEC,
): SelfTargetPreparation {
    val (sections, loadFailure) = loadStartupRootSections(timeoutSec)
    if (sections == null) {
        val failure = loadFailure ?: SelfTargetFailureDetail(SelfTargetFailureKind.Unknown, "snapshot load failed")
        return selfTargetPreparationFailure(failure.kind, failure.detail)
    }
    val update = buildCanonicalSelfUpdate(sections, selfPkg)
    if (update.writeRequired) {
        writeStartupCanonical(update.canonical, timeoutSec)?.let {
            return selfTargetPreparationFailure(SelfTargetFailureKind.ConfigWriteFailed, it)
        }
        RootSnapshotCache.invalidate()
    }
    cleanupLegacyConfigInputs(timeoutSec)
    val pmPackages = sections["pm_packages"]?.trimEnd()?.takeIf { it.isNotBlank() }
    val currentBootId = sections["current_boot_id"]?.trim()?.takeIf { it.isNotBlank() }
    VpnHideLog.d(TAG, "ensureSelfInTargets: done, selfNeedsRestart=${update.selfNeedsRestart}")
    return SelfTargetPreparation(
        rootAvailable = true,
        selfNeedsRestart = update.selfNeedsRestart,
        currentBootId = currentBootId,
        pmPackages = pmPackages,
    )
}

private data class CanonicalSelfUpdate(
    val canonical: CanonicalConfig,
    val selfNeedsRestart: Boolean,
    val writeRequired: Boolean,
)

private data class SelfTargetFailureDetail(
    val kind: SelfTargetFailureKind,
    val detail: String,
)

private fun loadStartupRootSections(timeoutSec: Long): Pair<Map<String, String>?, SelfTargetFailureDetail?> {
    val (exitCode, out) = suExec(buildRootShellSnapshotCommand(includePmPackages = true), timeoutSec = timeoutSec)
    if (exitCode != 0) {
        VpnHideLog.w(TAG, "ensureSelfInTargets: root snapshot failed (exit=$exitCode): ${out.trim()}")
        return null to SelfTargetFailureDetail(SelfTargetFailureKind.RootUnavailable, "exit=$exitCode")
    }
    return try {
        parseRootShellSnapshot(out).also(::validateRootSnapshotSections) to null
    } catch (e: RootSnapshotException) {
        // Root worked, but a required section was missing (incomplete probe — e.g.
        // a backend's control reply didn't come back). Distinct from no-root.
        VpnHideLog.w(TAG, "ensureSelfInTargets: snapshot parse failed: ${e.message}")
        null to SelfTargetFailureDetail(SelfTargetFailureKind.IncompleteData, e.message ?: "incomplete root snapshot")
    } catch (e: Exception) {
        VpnHideLog.w(TAG, "ensureSelfInTargets: snapshot parse failed: ${e.message}")
        null to SelfTargetFailureDetail(SelfTargetFailureKind.Unknown, e.message ?: "snapshot parse failed")
    }
}

private fun buildCanonicalSelfUpdate(
    sections: Map<String, String>,
    selfPkg: String,
): CanonicalSelfUpdate {
    val targets = parseTargetsSnapshot(RootSnapshot(sections))
    val baseCanonical =
        targets.canonicalConfig
            ?: buildCanonicalConfigFromTargetsSnapshot(targets, debug = false)
    val previousSelf = baseCanonical.apps[selfPkg]
    val selfNeedsRestart =
        previousSelf == null ||
            !previousSelf.java ||
            previousSelf.javaHooks != null ||
            previousSelf.native != NativeRole.All
    val updatedCanonical = canonicalConfigWithSelfTarget(baseCanonical, selfPkg)
    return CanonicalSelfUpdate(
        canonical = updatedCanonical,
        selfNeedsRestart = selfNeedsRestart,
        writeRequired = targets.canonicalConfig == null || updatedCanonical != baseCanonical,
    )
}

private fun writeStartupCanonical(
    canonical: CanonicalConfig,
    timeoutSec: Long,
): String? {
    val command =
        listOf(
            buildCanonicalConfigWriteCommand(canonical),
            ConfigChannels.reconcileCommand(),
            ConfigChannels.portsActivatorCommand(),
        ).joinToString(" ; ")
    val (exit, out) = suExec(command, timeoutSec = timeoutSec)
    if (exit == 0) return null
    VpnHideLog.w(TAG, "ensureSelfInTargets: canonical write failed (exit=$exit): ${out.trim()}")
    return "canonical write exit=$exit"
}

private fun cleanupLegacyConfigInputs(timeoutSec: Long) {
    val (exit, out) = suExec(buildLegacyConfigCleanupCommand(), timeoutSec = timeoutSec)
    if (exit != 0) {
        VpnHideLog.w(TAG, "legacy config cleanup failed (exit=$exit): ${out.trim()}")
    }
}

private fun selfTargetPreparationFailure(
    kind: SelfTargetFailureKind,
    error: String,
): SelfTargetPreparation =
    SelfTargetPreparation(
        rootAvailable = false,
        selfNeedsRestart = false,
        currentBootId = null,
        error = error,
        failureKind = kind,
    )
