package dev.okhsunrog.vpnhide.debug

import android.content.Context
import android.net.ConnectivityManager
import android.os.Build
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.DashboardCache
import dev.okhsunrog.vpnhide.DashboardState
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.LsposedConfig
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.collectDebugShellSnapshot
import dev.okhsunrog.vpnhide.collectHookCounterSnapshot
import dev.okhsunrog.vpnhide.core.CoreDiagnostics
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticSnapshot
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticsCache
import dev.okhsunrog.vpnhide.diagnostics.buildHookDiagnosticsText
import dev.okhsunrog.vpnhide.diagnostics.toDiagnosticInfo
import dev.okhsunrog.vpnhide.diagnostics.verdict
import dev.okhsunrog.vpnhide.loadDashboardState
import dev.okhsunrog.vpnhide.next
import dev.okhsunrog.vpnhide.readLsposedConfig
import dev.okhsunrog.vpnhide.statistics.buildStatisticsState
import dev.okhsunrog.vpnhide.toAgentStatisticsState
import dev.okhsunrog.vpnhide.ui.components.container
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.atomic.AtomicLong
import uniffi.vpnhide_core_ffi.CoreDiagnosticProbe as CoreProbe

private const val TAG = LogTags.TEST

internal data class DiagnosticFileEntry(
    val name: String,
    val file: File,
)

// ==========================================================================
//  Debug export — one canonical JSON
// ==========================================================================

/**
 * The single debug-export entry point behind the Diagnostics "Collect" modal. The
 * extended diagnostics and app inventory are always included. The ZIP carries
 * lightweight Java evidence; [attachKernelImage] and [attachJavaFiles] control
 * the two optional binary attachments.
 *
 * The outcome is explicit: a bundle whose self-test could not measure is still
 * [DebugExportOutcome.Written], carrying the reason. [DebugExportOutcome.Failed]
 * means collection or packaging itself threw and no file exists.
 */
internal suspend fun exportDebug(
    context: Context,
    selfNeedsRestart: Boolean,
    attachKernelImage: Boolean,
    attachJavaFiles: Boolean = false,
    exportPlan: String = "Explicit caller selection; extended diagnostics and installed apps included.",
): DebugExportOutcome =
    CanonicalConfigRepository.processScope
        .async(Dispatchers.IO) {
            try {
                val state = buildDebugState(context, selfNeedsRestart, StateContentOptions(forensics = true, appList = true))
                val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss_SSS", Locale.US).format(Date())
                val attachmentErrors = mutableListOf<String>()
                val file =
                    collectJavaArtifacts(context, attachJavaFiles).use { collected ->
                        if (collected.entries.isEmpty()) attachmentErrors += collected.status
                        val java = collected.copy(status = "$exportPlan\n${collected.status}")
                        if (attachKernelImage) {
                            // Packaging tens of MB of partition images can fail on its own (a
                            // full /data); it reports that as a missing file, not an exception.
                            writeKernelBundleZip(context, timestamp, state.toJson(), java)
                                ?: File(context.cacheDir, "vpnhide_debug_$timestamp.zip").also {
                                    attachmentErrors += "Kernel packaging failed; baseline preserved."
                                    writeDebugArchive(
                                        it,
                                        mapOf(
                                            "state.json" to state.toJson(),
                                            "kernel/collection.txt" to "Kernel packaging failed; baseline preserved.",
                                        ),
                                        emptyList(),
                                        java,
                                    )
                                }
                        } else {
                            // Bundle the single state.json in a .zip too: forums/messengers (4pda,
                            // the main report channel) whitelist .zip but reject a bare .json, and
                            // the JSON compresses ~10x. Every export kind is now a .zip carrying
                            // state.json, so the caller has one format to handle.
                            File(context.cacheDir, "vpnhide_debug_$timestamp.zip").also {
                                writeDebugArchive(it, mapOf("state.json" to state.toJson()), emptyList(), java)
                            }
                        }
                    }
                DebugExportOutcome.Written(file, state.errors + attachmentErrors, archiveSummary(file))
            } catch (c: CancellationException) {
                throw c
            } catch (e: Exception) {
                VpnHideLog.e(TAG, "Debug export failed", e)
                DebugExportOutcome.Failed(e.message ?: e.javaClass.simpleName)
            }
        }.await()

@Suppress("LongMethod")
private suspend fun buildDebugState(
    context: Context,
    selfNeedsRestart: Boolean,
    options: StateContentOptions,
): VpnHideState {
    // Keep the existing scoped capture lease and restoration. Export verbosity
    // does not change the persistent user logging preference.
    val beforeCapture = beforeDebugCapture()
    val loggingSession = if (options.forensics) beginDebugCaptureLogging() else null
    var restoreAttempted = false
    val errors = mutableListOf<String>()
    return try {
        val counterBaseline = if (options.forensics) collectHookCounterSnapshot() else null
        // Retained Core evidence: no post-baseline measurement or fresh suite is claimed.
        // Authoritative module/liveness state — the SAME snapshot the dashboard
        // derives from. This is what fixes the old export path silently reading
        // "inactive" from a shell that never emitted proc_exists/ports_chain.
        val refreshedRoot =
            dev.okhsunrog.vpnhide.diagnostics.readAutonomousRoot(
                read = { RootSnapshotCache.refresh() },
                onFailure = { errors += "root snapshot failed: ${it.message}" },
            )

        val diagnosticSnapshot = DiagnosticsCache.awaitSnapshot(refreshedRoot)
        val rootSnapshot = diagnosticSnapshot.rootSnapshot ?: RootSnapshot(emptyMap())
        val dashboard = captureDashboard(context, selfNeedsRestart, rootSnapshot, diagnosticSnapshot, errors)

        val shellSnapshot =
            if (options.forensics) {
                collectDebugShellSnapshot().also {
                    if (it.exitCode != 0) errors += "debug shell exit=${it.exitCode}"
                    it.sections["debug_snapshot_truncated"]?.let { s -> errors += "snapshot truncated at: $s" }
                }
            } else {
                null
            }
        // Independent raw forensic observation of this existing app process.
        // It cannot promote the retained Core run or claim fresh hiding proof.
        val networkView =
            if (options.forensics) {
                runCatching { captureSelfNetworkView(context, expectHidden = false) }
                    .onFailure { errors += "network view: ${it.message}" }
                    .getOrNull()
            } else {
                null
            }
        val dmesg = if (options.forensics) captureDmesg() else ""
        val logcat = if (options.forensics) captureDebugLogcat().ifEmpty { "(no logcat entries)" } else ""
        val session = loggingSession?.let { it.withRestore(restoreDebugCaptureLogging(it)) }
        restoreAttempted = true

        buildVpnHideState(
            context = context,
            captureKind = "debug",
            generatedAt = isoNow(),
            selfNeedsRestart = selfNeedsRestart,
            rootSnapshot = rootSnapshot,
            shellSnapshot = shellSnapshot,
            gate = null,
            checkResults = null,
            autonomousSnapshot = diagnosticSnapshot,
            dashboard = dashboard,
            beforeCapture = beforeCapture,
            dmesg = dmesg,
            logcat = logcat,
            bootLsposedLogcat = if (options.forensics) captureBootLsposedLogcat() else "",
            lsposedConfigDb = if (options.forensics) buildLsposedConfigText(context) else "",
            hookReport = shellSnapshot?.let { buildHookDiagnosticsText(context, it, counterBaseline) },
            // Always carry the point-in-time hook counters (a few KB, already in the
            // snapshot): the per-uid SOCKET_BIND_INTERFACE deny-count is what tells a
            // dead redirect from a working one, and a bundle that silently dropped it
            // is exactly the blind spot this closes. Not gated behind forensics.
            statistics =
                runCatching {
                    buildStatisticsState(rootSnapshot).toAgentStatisticsState(selfPackage = context.packageName)
                }.getOrNull(),
            debugCapture = session?.toDebugCaptureInfo(),
            errors = errors,
            networkView = networkView,
            options = options,
        )
    } finally {
        if (!restoreAttempted) {
            loggingSession?.let { restoreDebugCaptureLogging(it) }
        }
    }
}

/** Dashboard failure must not discard the run evidence or the rest of the archive. */
private suspend fun captureDashboard(
    context: Context,
    selfNeedsRestart: Boolean,
    root: RootSnapshot,
    snapshot: AutonomousDiagnosticSnapshot,
    errors: MutableList<String>,
): DashboardState? =
    try {
        if (snapshot.rootJoined) loadDashboardState(context, selfNeedsRestart, root, snapshot) else null
    } catch (c: CancellationException) {
        throw c
    } catch (e: Exception) {
        errors += "dashboard assembly failed: ${e.message}"
        null
    }

private fun beforeDebugCapture(): BeforeCaptureInfo? =
    runCatching {
        val display = DashboardCache.presentation.value
        val dashboard = display.state
        val operation = display.diagnostics.sources.config
        BeforeCaptureInfo(
            observedAt = isoNow(),
            diagnostics = null,
            autonomousDiagnostics = display.diagnostics.toDiagnosticInfo(),
            protection = dashboard?.protection,
            messages = dashboard?.messages.orEmpty(),
            pendingOperationIds = operation.operations.sorted(),
            operationRechecking = operation.rechecking,
            lastOperationId = operation.lastResult?.id,
            lastOperationPhases =
                operation.lastResult
                    ?.phases
                    .orEmpty()
                    .mapKeys { it.key.name }
                    .mapValues { it.value.name },
            lastOperationFailure = operation.lastResult?.failure?.name,
        )
    }.getOrNull()

/** The app's own post-Binder network view, PendingIntent path included (this is a real app process). */
internal fun captureSelfNetworkView(
    context: Context,
    expectHidden: Boolean,
): NetworkViewSnapshot =
    captureNetworkView(
        context = context,
        cm = context.getSystemService(ConnectivityManager::class.java),
        uid = android.os.Process.myUid(),
        packageName = context.packageName,
        options = NetworkViewOptions(expectHidden = expectHidden, includePendingIntent = true),
    )

/** ISO-8601 timestamp for [VpnHideState.generatedAt] (the serializer has no clock). */
internal fun isoNow(): String = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ssZ", Locale.US).format(Date())

/** The app's and its native components' log tags (core's fixed `LogcatAppTags` set). */
private fun captureDebugLogcat(): String {
    val result = CoreRoot.collectOneBlocking(CoreProbe.LogcatAppTags)
    return if (result.ok) result.text else "(logcat failed: core exit=${result.exit})\n${result.text}"
}

/** This boot's dmesg ring as core reads it; empty when core is unavailable. */
internal fun captureDmesg(): String = CoreRoot.collectOneBlocking(CoreProbe.Dmesg).text

/** Clear the kernel ring (`dmesg -c` in core, behind its explicit `clear_dmesg` flag). */
internal fun clearDmesg(): Boolean =
    (CoreRoot.collectBlocking(emptyList(), clearDmesg = true) as? CoreDiagnostics.Collected)?.dmesgCleared == true

internal fun captureBootLsposedLogcat(): String {
    val result = CoreRoot.collectOneBlocking(CoreProbe.LogcatLsposedBoot)
    return buildString {
        appendLine("commandExit=${result.exit}")
        appendLine("source=logcat -d -b all -v threadtime")
        appendLine("scope=best_effort_current_ring_buffer")
        appendLine("note=Contains boot-time LSPosed/Vector context only if the logcat ring buffer has not rotated yet.")
        appendLine("patterns=core LogcatLsposedBoot probe (LSPosed/Vector/Xposed tags and this package)")
        appendLine()
        appendLine(result.text.ifBlank { "(no LSPosed/Vector boot logcat entries in current buffers)" }.trimEnd())
    }.trimEnd()
}

internal fun appVersionText(context: Context): String =
    try {
        val pInfo = context.packageManager.getPackageInfo(context.packageName, 0)
        val code =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                pInfo.longVersionCode.toString()
            } else {
                @Suppress("DEPRECATION")
                pInfo.versionCode.toString()
            }
        "${pInfo.versionName} ($code)"
    } catch (_: Exception) {
        "(unknown)"
    }

internal fun buildLsposedConfigText(context: Context): String {
    val config =
        runCatching {
            readLsposedConfig(context, context.packageName)
        }.getOrNull()
            ?: return "(not available)"
    return when (config) {
        LsposedConfig.ModuleNotConfigured -> {
            "module=not_configured"
        }

        LsposedConfig.Disabled -> {
            "module=disabled"
        }

        is LsposedConfig.Enabled -> {
            buildString {
                appendLine("module=enabled")
                appendLine("hasSystemFramework=${config.hasSystemFramework}")
                appendLine("scope=${config.entries.joinToString()}")
                appendLine("extraScope=${config.extraEntries.joinToString()}")
            }.trimEnd()
        }
    }
}
