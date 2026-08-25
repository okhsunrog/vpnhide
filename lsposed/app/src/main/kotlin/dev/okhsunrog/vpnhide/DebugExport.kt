package dev.okhsunrog.vpnhide

import android.content.Context
import android.net.ConnectivityManager
import android.os.Build
import dev.okhsunrog.vpnhide.statistics.buildStatisticsState
import dev.okhsunrog.vpnhide.ui.components.container
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

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
 * [options] (forensics / app-list) drive the JSON content — the SAME type the agent
 * bridge getState takes — and [attachKernelImage] decides the container: a plain
 * `.json`, or a `.zip` bundling the boot/kernel images next to that same state.json.
 */
internal suspend fun exportDebug(
    cm: ConnectivityManager,
    context: Context,
    selfNeedsRestart: Boolean,
    options: StateContentOptions,
    attachKernelImage: Boolean,
): File? =
    withContext(Dispatchers.IO) {
        try {
            val state = buildDebugState(cm, context, selfNeedsRestart, options)
            val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date())
            if (attachKernelImage) {
                writeKernelBundleZip(context, timestamp, state.toJson())
            } else {
                // Bundle the single state.json in a .zip too: forums/messengers (4pda,
                // the main report channel) whitelist .zip but reject a bare .json, and
                // the JSON compresses ~10x. Every export kind is now a .zip carrying
                // state.json, so the caller has one format to handle.
                File(context.cacheDir, "vpnhide_debug_$timestamp.zip").also {
                    writeDiagnosticZip(it, mapOf("state.json" to state.toJson()))
                }
            }
        } catch (c: CancellationException) {
            throw c
        } catch (e: Exception) {
            VpnHideLog.e(TAG, "Debug export failed", e)
            null
        }
    }

@Suppress("LongMethod")
private suspend fun buildDebugState(
    cm: ConnectivityManager,
    context: Context,
    selfNeedsRestart: Boolean,
    options: StateContentOptions,
): VpnHideState {
    // Forensics forces debug logging + a fresh dmesg window while the capture runs;
    // a lean (no-forensics) export skips all of that and just serializes the state.
    val loggingSession = if (options.forensics) beginDebugCaptureLogging() else null
    var restoreAttempted = false
    val errors = mutableListOf<String>()
    return try {
        val counterBaseline = if (options.forensics) collectHookCounterSnapshot() else null
        // Clear dmesg so we only capture output from the hooks the checks fire.
        if (options.forensics) suExec("dmesg -c > /dev/null 2>&1")
        val checkResults = runAllChecks(cm, context)

        // Authoritative module/liveness state — the SAME snapshot the dashboard
        // derives from. This is what fixes the old export path silently reading
        // "inactive" from a shell that never emitted proc_exists/ports_chain.
        val rootSnapshot =
            runCatching { RootSnapshotCache.refresh() }
                .getOrElse {
                    errors += "root snapshot failed: ${it.message}"
                    RootSnapshot(emptyMap())
                }

        val shellSnapshot =
            if (options.forensics) {
                collectDebugShellSnapshot().also {
                    if (it.exitCode != 0) errors += "debug shell exit=${it.exitCode}"
                    it.sections["debug_snapshot_truncated"]?.let { s -> errors += "snapshot truncated at: $s" }
                }
            } else {
                null
            }
        val dmesg = if (options.forensics) suExec("dmesg 2>/dev/null").second else ""
        val logcat = if (options.forensics) captureDebugLogcat().ifEmpty { "(no logcat entries)" } else ""
        val session = loggingSession?.let { it.withRestore(restoreDebugCaptureLogging(it)) }
        restoreAttempted = true

        // Same gate the collect-warning shows, off the snapshot just refreshed above.
        val gate = captureGateFrom(rootSnapshot, context, selfNeedsRestart)
        buildVpnHideState(
            context = context,
            captureKind = "debug",
            generatedAt = isoNow(),
            selfNeedsRestart = selfNeedsRestart,
            rootSnapshot = rootSnapshot,
            shellSnapshot = shellSnapshot,
            gate = gate,
            checkResults = checkResults,
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
            options = options,
        )
    } finally {
        if (!restoreAttempted) {
            loggingSession?.let { restoreDebugCaptureLogging(it) }
        }
    }
}

/** ISO-8601 timestamp for [VpnHideState.generatedAt] (the serializer has no clock). */
internal fun isoNow(): String = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ssZ", Locale.US).format(Date())

/**
 * The gate that decides whether a debug/logcat capture taken *right now* would be
 * meaningful, computed from a root snapshot. The ONE place both the export and the
 * pre-collect warning derive it, so they can never disagree. Cheap and stateless —
 * a VPN-iface read plus one self-routing probe — and independent of the
 * once-per-process [DiagnosticsCache]; that cache freezes its verdict on the first
 * run (typically with the VPN up), which is exactly why it must NOT back a "is a
 * capture worth taking now" check.
 */
internal fun captureGateFrom(
    snapshot: RootSnapshot,
    context: Context,
    selfNeedsRestart: Boolean,
): DiagnosticGate =
    resolveDiagnosticGate(
        vpnActive = isVpnActiveFromSnapshot(snapshot.sections["vpn_ifaces"].orEmpty()),
        selfRouted = GroundTruthProbe.selfRoutedThroughVpn(context),
        selfNeedsRestart = selfNeedsRestart,
    )

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

private fun captureDebugLogcat(): String {
    val tags =
        listOf(
            "VPNHideTest:*",
            "VpnHide:*",
            "VpnHide-Dashboard:*",
            "VpnHide-Startup:*",
            "VpnHide-LSPosed:*",
            "VpnHide-Diag:*",
            "VpnHide-Logcat:*",
            "VpnHide-Update:*",
            "VpnHideAgentBridge:*",
            "vpnhide:*",
            "vpnhide_ports:*",
            "vpnhide-zygisk:*",
            "shadowhook_tag:*",
        ).joinToString(" ")
    val (exit, output) = suExec("logcat -d -b all -v threadtime -s $tags 2>/dev/null")
    return if (exit == 0) output else "(logcat failed: exit=$exit)\n$output"
}

internal fun captureBootLsposedLogcat(): String {
    val (exit, output) = suExec(buildBootLsposedLogcatCommand(), timeoutSec = 15)
    return buildString {
        appendLine("commandExit=$exit")
        appendLine("source=logcat -d -b all -v threadtime")
        appendLine("scope=best_effort_current_ring_buffer")
        appendLine("note=Contains boot-time LSPosed/Vector context only if the logcat ring buffer has not rotated yet.")
        appendLine("patterns=${BOOT_LSPOSED_LOGCAT_PATTERNS.joinToString(",")}")
        appendLine()
        appendLine(output.ifBlank { "(no LSPosed/Vector boot logcat entries in current buffers)" }.trimEnd())
    }.trimEnd()
}

internal fun buildBootLsposedLogcatCommand(): String {
    val pattern = BOOT_LSPOSED_LOGCAT_PATTERNS.joinToString("|")
    return """
        logcat -d -b all -v threadtime 2>/dev/null |
          grep -Ei '$pattern' |
          tail -2000 || true
        """.trimIndent()
}

private val BOOT_LSPOSED_LOGCAT_PATTERNS =
    listOf(
        LogTags.LSPOSED,
        "LSPosed-Bridge",
        "VectorNative",
        "VectorBridge",
        "LSPosedService",
        "LSPlt",
        "LSPHooker",
        "LSPosedBridge",
        "Xposed",
        "org[.]lsposed",
        "lspd",
        "modules_config",
        "dev[.]okhsunrog[.]vpnhide",
    )

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
