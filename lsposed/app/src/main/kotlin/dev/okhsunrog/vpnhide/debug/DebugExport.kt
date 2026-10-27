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
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticSnapshot
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticsCache
import dev.okhsunrog.vpnhide.diagnostics.buildHookDiagnosticsText
import dev.okhsunrog.vpnhide.diagnostics.diagnosticSummary
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
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import uniffi.vpnhide_core_ffi.CoreDiagnosticProbe as CoreProbe

private const val TAG = LogTags.TEST

internal data class DiagnosticFileEntry(
    val name: String,
    val file: File,
)

/** Capture identity: unique per export, so a capture's request can never join an earlier suite. */
private val nextDebugCaptureId = AtomicLong(1L)

// ==========================================================================
//  Debug export — one canonical JSON
// ==========================================================================

/**
 * The single debug-export entry point behind the Diagnostics "Collect" modal. The
 * [options] (forensics / app-list) drive the JSON content — the SAME type the agent
 * bridge getState takes — and [attachKernelImage] decides the container: a plain
 * `.json`, or a `.zip` bundling the boot/kernel images next to that same state.json.
 *
 * The outcome is explicit: a bundle whose self-test could not measure is still
 * [DebugExportOutcome.Written], carrying the reason. [DebugExportOutcome.Failed]
 * means collection or packaging itself threw and no file exists.
 */
internal suspend fun exportDebug(
    context: Context,
    selfNeedsRestart: Boolean,
    options: StateContentOptions,
    attachKernelImage: Boolean,
): DebugExportOutcome =
    CanonicalConfigRepository.processScope
        .async(Dispatchers.IO) {
            try {
                val state = buildDebugState(context, selfNeedsRestart, options)
                val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date())
                val file =
                    if (attachKernelImage) {
                        // Packaging tens of MB of partition images can fail on its own (a
                        // full /data); it reports that as a missing file, not an exception.
                        writeKernelBundleZip(context, timestamp, state.toJson())
                            ?: return@async DebugExportOutcome.Failed("kernel image packaging produced no archive")
                    } else {
                        // Bundle the single state.json in a .zip too: forums/messengers (4pda,
                        // the main report channel) whitelist .zip but reject a bare .json, and
                        // the JSON compresses ~10x. Every export kind is now a .zip carrying
                        // state.json, so the caller has one format to handle.
                        File(context.cacheDir, "vpnhide_debug_$timestamp.zip").also {
                            writeDiagnosticZip(it, mapOf("state.json" to state.toJson()))
                        }
                    }
                DebugExportOutcome.Written(file, state.errors)
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
    // Forensics forces debug logging + a fresh dmesg window while the capture runs;
    // a lean (no-forensics) export skips all of that and just serializes the state.
    val beforeCapture = beforeDebugCapture()
    val loggingSession = if (options.forensics) beginDebugCaptureLogging() else null
    var restoreAttempted = false
    val errors = mutableListOf<String>()
    return try {
        val counterBaseline = if (options.forensics) collectHookCounterSnapshot() else null
        // Clear dmesg so we only capture output from the hooks the checks fire.
        if (options.forensics) clearDmesg()
        // The suite belongs to the run coordinator, not to the export: this is a fresh
        // capture-identified run that cannot join one whose probes started before the
        // logging and counter baseline above. Its terminal attempt is recorded either
        // way — a blocked, interrupted or failed run is evidence, not a dead export.
        val selfTest =
            debugSelfTestFrom(
                DiagnosticsCache.captureRun(context, selfNeedsRestart, nextDebugCaptureId.getAndIncrement()),
            )
        errors += selfTest.errors

        // Authoritative module/liveness state — the SAME snapshot the dashboard
        // derives from. This is what fixes the old export path silently reading
        // "inactive" from a shell that never emitted proc_exists/ports_chain.
        val refreshedRoot =
            runCatching { RootSnapshotCache.refresh() }
                .getOrElse {
                    errors += "root snapshot failed: ${it.message}"
                    null
                }

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
        // The post-Binder network model as this uid sees it. Taken after the suite
        // so its callback window does not overlap the suite's own callback probe;
        // the VPN is expected hidden only when the run itself was ROUTED.
        val networkView =
            if (options.forensics) {
                runCatching { captureSelfNetworkView(context, selfTest.gate == DiagnosticGate.ROUTED) }
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
            // The gate is the run's own outcome now, never a second independent probe:
            // only a completed run is ROUTED, and an unmeasurable one says why in errors.
            gate = selfTest.gate,
            checkResults = selfTest.checkResults,
            selfTestRunId = selfTest.runId,
            // The projection after the capture's own run settled: what the screens show now.
            diagnostics = diagnosticSummary(diagnosticSnapshot),
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
    snapshot: DiagnosticSnapshot,
    errors: MutableList<String>,
): DashboardState? =
    try {
        loadDashboardState(context, selfNeedsRestart, root, snapshot.presentation)
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
            diagnostics = diagnosticSummary(display.diagnostics),
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

/**
 * Pack a ZIP: named text entries + raw file entries. The one packaging primitive for
 * every debug export — the plain debug export (just `state.json`), the kernel-image
 * export (+ binary partition images), and the full-logcat recorder (+ a multi-MB raw
 * log). Every bundle is a `.zip` carrying the canonical `state.json`; the heavy
 * variants add their payload alongside it.
 */
internal fun writeDiagnosticZip(
    zipFile: File,
    textEntries: Map<String, String>,
    fileEntries: List<DiagnosticFileEntry> = emptyList(),
) {
    ZipOutputStream(zipFile.outputStream()).use { zos ->
        for ((name, content) in textEntries) {
            zos.putNextEntry(ZipEntry(name))
            zos.write(content.toByteArray())
            zos.closeEntry()
        }
        for ((name, file) in fileEntries) {
            zos.putNextEntry(ZipEntry(name))
            file.inputStream().use { it.copyTo(zos) }
            zos.closeEntry()
        }
    }
}

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
