package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreDiagnostics
import dev.okhsunrog.vpnhide.core.CoreProbeResult
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.core.CoreRootEffect
import dev.okhsunrog.vpnhide.generated.IfaceLists
import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot
import uniffi.vpnhide_core_ffi.coreParsePackageUidMap
import java.io.File
import uniffi.vpnhide_core_ffi.CoreDiagnosticProbe as CoreProbe

private const val TAG = LogTags.APP

// The kmod's folded control+stats node (docs/protocol.md §OPEN-4): a write is a
// `vpnhide 2 config` snapshot, a read returns telemetry-v1 status+stats.
// Replaces the old /proc/vpnhide_targets (decimal UID list) +
// /proc/vpnhide_debug nodes.
internal const val PROC_CTL = "/proc/vpnhide_ctl"

// Diagnostic-only companion to PROC_CTL (kmod/vpnhide_kmod.c, vpnhide_diag_show):
// human-readable hook-mask/nmissed/live-is_vpn_ifname dump for field debugging.
// Never part of the frozen control/telemetry wire and never parsed — the debug
// bundle just cats it verbatim, same as kmod_state below.
internal const val PROC_DIAG = "/proc/vpnhide_diag"

// The required common module: core-daemon, the only root process and the only
// writer of config, control nodes, Zygisk targets and Ports rules.
internal const val CORE_MODULE_DIR = "/data/adb/modules/vpnhide_core"
internal const val KMOD_MODULE_DIR = "/data/adb/modules/vpnhide_kmod"

// The built-in kernel backend (CONFIG_VPNHIDE=y) has no module at all: the
// driver is part of the kernel image and core configures it over the same
// /proc/vpnhide_ctl channel as the .ko; the `backend 0x4` line in the control
// status reply is what tells it apart from the loadable .ko (0x0).
internal const val ZYGISK_MODULE_DIR = "/data/adb/modules/vpnhide_zygisk"
internal const val APP_PACKAGE_NAME = "dev.okhsunrog.vpnhide"
internal const val ZYGISK_STATUS_FILE_NAME = "vpnhide_zygisk_active"
internal const val ZYGISK_STATUS_FILE = "/data/user/0/dev.okhsunrog.vpnhide/files/vpnhide_zygisk_active"

// KPM (KernelPatch Module) backend. The KPM has no /proc node; its runtime
// channel is the KernelPatch ctl0 supercall, which core reads for the snapshot's
// `kpm_state` section.
internal const val KPM_MODULE_DIR = "/data/adb/modules/vpnhide_kpm"

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

/**
 * Parse `pm list packages -U` output into `package → uids`. Both the plain
 * and `-f` path-prefixed forms are accepted. Repeated per-user rows and
 * comma-separated `--user all` rows are unioned.
 */
internal fun parsePackageUidMap(raw: String): Map<String, List<Int>> = coreParsePackageUidMap(raw)

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

/**
 * Why startup root-state preparation failed, so the UI can tell the user the
 * truth instead of always blaming root permissions. The distinction is known at
 * the point of failure (not parsed back out of a string).
 */
internal enum class SelfTargetFailureKind {
    /** Core did not answer: the VPN Hide Core module is missing, disabled or not running. */
    RootUnavailable,

    /** Core answered but speaks another protocol version: the app and the core module
     *  come from different releases, so one of them must be updated. */
    CoreVersionMismatch,

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
    val error: String? = null,
    val failureKind: SelfTargetFailureKind = SelfTargetFailureKind.Unknown,
)

/** This boot's id. World-readable; no root needed. */
internal fun readBootId(): String = runCatching { File("/proc/sys/kernel/random/boot_id").readText().trim() }.getOrDefault("")

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
            ?: readBootId()
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
 * Observe Core's managed target preparation, requesting its fixed command when
 * still needed. Core owns boot bootstrap, legacy inputs and durable outcomes.
 * A changed managed role conservatively retains the current-process restart hint.
 * Called once at app startup; result is shared with all screens.
 */
internal suspend fun ensureSelfInTargets(selfPkg: String): SelfTargetPreparation {
    val (sections, loadFailure) = loadStartupRootSections()
    if (sections == null) {
        val failure = loadFailure ?: (SelfTargetFailureKind.Unknown to "snapshot load failed")
        return selfTargetPreparationFailure(failure.first, failure.second)
    }
    val targets = parseTargetsSnapshot(joinRootSnapshot(sections, CoreSessions.updates.value))
    val before = targets.canonicalConfig
    val needsPreparation = before == null || canonicalConfigWithSelfTarget(before, selfPkg) != before
    if (needsPreparation) {
        val result =
            CanonicalConfigRepository.commit(
                CanonicalMutation(
                    emptyList(),
                    bootstrap = true,
                    source = OperationSource.System,
                    coupledEffects = listOf(CoreRootEffect.PrepareSelfTargets),
                    protectAllDrafts = true,
                ),
            )
        if (!result.succeeded) {
            return selfTargetPreparationFailure(
                SelfTargetFailureKind.ConfigWriteFailed,
                "Core self-target preparation: ${result.output.trim()}",
            )
        }
    }
    val currentBootId = sections["current_boot_id"]?.trim()?.takeIf { it.isNotBlank() }
    VpnHideLog.d(TAG, "ensureSelfInTargets: done, selfNeedsRestart=$needsPreparation")
    return SelfTargetPreparation(
        rootAvailable = true,
        selfNeedsRestart = needsPreparation,
        currentBootId = currentBootId,
    )
}

/**
 * Why the startup root snapshot did not come back, or null when it did. Core not
 * reachable is the "install/enable the core module" gate; a protocol skew is the
 * "update the lagging component" gate; core reached but the probe failing is a
 * retryable core-side failure, not a missing module.
 */
internal fun startupProbeFailure(diagnostics: CoreDiagnostics): Pair<SelfTargetFailureKind, String>? =
    when (diagnostics) {
        is CoreDiagnostics.Failed -> {
            val kind =
                if (diagnostics.status == "protocol_skew") {
                    SelfTargetFailureKind.CoreVersionMismatch
                } else {
                    SelfTargetFailureKind.RootUnavailable
                }
            kind to listOf(diagnostics.status, diagnostics.detail).filter { it.isNotBlank() }.joinToString(": ")
        }

        is CoreDiagnostics.Collected -> {
            val result = diagnostics.results.firstOrNull() ?: CoreProbeResult.Unavailable
            when {
                result.truncated -> {
                    SelfTargetFailureKind.IncompleteData to "core root snapshot probe output truncated"
                }

                result.ok -> {
                    null
                }

                else -> {
                    SelfTargetFailureKind.Unknown to
                        listOf("core root snapshot probe exit=${result.exit}", result.text.take(512).trim())
                            .filter { it.isNotBlank() }
                            .joinToString(": ")
                }
            }
        }
    }

internal suspend fun loadStartupRootSections(
    load: suspend () -> RootSnapshot = RootSnapshotCache::getOrLoad,
): Pair<Map<String, String>?, Pair<SelfTargetFailureKind, String>?> =
    try {
        load().sections to null
    } catch (e: kotlinx.coroutines.CancellationException) {
        throw e
    } catch (e: Exception) {
        val rootFailure = (e as? ObservationReadException)?.cause as? RootSnapshotException ?: e as? RootSnapshotException
        null to ((rootFailure?.kind ?: SelfTargetFailureKind.Unknown) to (e.message ?: "snapshot read failed"))
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
