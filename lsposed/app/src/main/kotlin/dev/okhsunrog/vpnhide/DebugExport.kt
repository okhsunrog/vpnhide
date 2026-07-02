package dev.okhsunrog.vpnhide

import android.content.Context
import android.net.ConnectivityManager
import android.os.Build
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

private const val TAG = "VPNHideTest"

internal data class DiagnosticFileEntry(
    val name: String,
    val file: File,
)

// ==========================================================================
//  Debug log export
// ==========================================================================

internal suspend fun exportDebugZip(
    cm: ConnectivityManager,
    context: Context,
    selfNeedsRestart: Boolean,
): File? =
    withContext(Dispatchers.IO) {
        // Force-enable debug logging across app, system_server and active
        // native sinks while the capture runs. The helper records exactly what
        // it applied/restored so bug reports can distinguish "no logs" from
        // "debug propagation failed".
        val loggingSession = beginDebugCaptureLogging(context)
        var restoreAttempted = false
        try {
            val counterBaseline = collectHookCounterSnapshot()
            // 1. Clear dmesg so we only capture fresh output from the
            //    native hooks fired by runAllChecks below.
            suExec("dmesg -c > /dev/null 2>&1")

            // 2. Run all diagnostic checks (this triggers native hooks)
            val checkResults = runAllChecks(cm, context)

            // 3. Capture dmesg right after checks
            val (_, dmesg) = suExec("dmesg 2>/dev/null")
            val shellSnapshot = collectDebugShellSnapshot()

            val logcat = captureDebugLogcat()
            val restore = restoreDebugCaptureLogging(context, loggingSession)
            restoreAttempted = true
            val completedLoggingSession = loggingSession.withRestore(restore)

            // 4. Collect everything into named files — each section is its own
            //    builder below.
            val files =
                linkedMapOf(
                    "summary.txt" to
                        buildDiagnosticSummaryText(
                            context = context,
                            selfNeedsRestart = selfNeedsRestart,
                            results = checkResults,
                            shellSnapshot = shellSnapshot,
                            loggingSession = completedLoggingSession,
                            captureKind = "debug_zip",
                        ),
                    "diagnostics.txt" to buildDiagnosticsText(checkResults),
                )
            files.putAll(
                buildCommonDiagnosticTextFiles(
                    context = context,
                    selfNeedsRestart = selfNeedsRestart,
                    shellSnapshot = shellSnapshot,
                    loggingSession = completedLoggingSession,
                ),
            )
            files["hook_report.txt"] =
                buildHookDiagnosticsText(
                    context = context,
                    shellSnapshot = shellSnapshot,
                    counterBaseline = counterBaseline,
                    results = checkResults,
                )
            files["dmesg_vpnhide.txt"] = filterVpnHideDmesg(dmesg)
            files["dmesg_full.txt"] = dmesg.ifBlank { "(no dmesg entries)" }
            files["logcat_vpnhide.txt"] = logcat.ifEmpty { "(no logcat entries)" }

            // Create zip
            val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date())
            val zipFile = File(context.cacheDir, "vpnhide_debug_$timestamp.zip")
            writeDiagnosticZip(zipFile, files)
            zipFile
        } catch (e: Exception) {
            Log.e(TAG, "Debug export failed", e)
            null
        } finally {
            if (!restoreAttempted) {
                restoreDebugCaptureLogging(context, loggingSession)
            }
        }
    }

// ── Debug-zip section builders (each produces one file in the export) ─────

private fun badge(passed: Boolean?): String =
    when (passed) {
        true -> "PASS"
        false -> "FAIL"
        null -> "INFO"
    }

internal fun buildDiagnosticSummaryText(
    context: Context,
    selfNeedsRestart: Boolean,
    results: CheckResults?,
    shellSnapshot: DebugShellSnapshot,
    loggingSession: DebugCaptureLoggingSession,
    captureKind: String,
): String =
    buildString {
        appendLine("Diagnostic bundle schema: 3")
        appendLine("Capture type: $captureKind")
        appendLine("Generated: ${SimpleDateFormat("yyyy-MM-dd HH:mm:ss Z", Locale.US).format(Date())}")
        appendLine("App package: ${context.packageName}")
        appendLine("App version: ${appVersionText(context)}")
        if (results == null) {
            appendLine("Diagnostics: not run")
        } else {
            val score = results.all.score()
            appendLine("Diagnostics: ${score.passed}/${score.total} passed")
        }
        appendLine("selfNeedsRestart: $selfNeedsRestart")
        appendLine("debugCaptureForced: ${loggingSession.forced}")
        appendLine("debugCaptureApplyExit: ${loggingSession.apply.commandExit?.toString() ?: "(n/a)"}")
        appendLine("debugCaptureRestoreExit: ${loggingSession.restore?.commandExit?.toString() ?: "(n/a)"}")
        appendLine("rootSnapshotExit: ${shellSnapshot.exitCode}")
        shellSnapshot.section("debug_snapshot_error").takeIf { it.isNotBlank() }?.let {
            appendLine("rootSnapshotError: $it")
        }
        appendLine()
        appendDebugSection("Current boot_id", shellSnapshot.section("current_boot_id"))
        appendDebugSection("Backend evidence", buildBackendEvidence(shellSnapshot))
    }

internal fun buildCommonDiagnosticTextFiles(
    context: Context,
    selfNeedsRestart: Boolean,
    shellSnapshot: DebugShellSnapshot,
    loggingSession: DebugCaptureLoggingSession,
): LinkedHashMap<String, String> =
    linkedMapOf(
        "device_info.txt" to buildDeviceInfoText(context, selfNeedsRestart, shellSnapshot),
        "backends.txt" to buildBackendsText(context, shellSnapshot),
        "modules.txt" to buildModulesText(shellSnapshot),
        "config.txt" to buildConfigText(shellSnapshot),
        "interfaces.txt" to buildInterfacesText(shellSnapshot),
        "proc_net.txt" to buildProcNetText(shellSnapshot),
        "kernel.txt" to buildKernelText(shellSnapshot),
        "kernel_partitions.txt" to buildKernelPartitionMetadataText(),
        "boot_logcat_lsposed.txt" to captureBootLsposedLogcat(),
        "debug_capture.txt" to loggingSession.toText(),
    )

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

private fun buildDiagnosticsText(results: CheckResults): String =
    buildString {
        val score = results.all.score()
        appendLine("=== Diagnostics: ${score.passed}/${score.total} passed ===")
        appendLine()
        appendLine("--- Native level ---")
        for (c in results.nativeAll) {
            appendLine("[${badge(c.passed)}] ${c.name}")
            appendLine("  ${c.detail}")
        }
        appendLine()
        appendLine("--- Java API level ---")
        for (c in results.java) {
            appendLine("[${badge(c.passed)}] ${c.name}")
            appendLine("  ${c.detail}")
        }
    }

private fun buildDeviceInfoText(
    context: Context,
    selfNeedsRestart: Boolean,
    shellSnapshot: DebugShellSnapshot,
): String =
    buildString {
        appendLine("Device: ${Build.MANUFACTURER} ${Build.MODEL}")
        appendLine("Android: ${Build.VERSION.RELEASE} (SDK ${Build.VERSION.SDK_INT})")
        appendLine("ABI: ${Build.SUPPORTED_ABIS.joinToString()}")
        appendLine("App package: ${context.packageName}")
        appendLine("App version: ${appVersionText(context)}")
        appendLine("selfNeedsRestart: $selfNeedsRestart")
        appendLine()
        appendDebugSection("uname", shellSnapshot.section("uname"))
        appendDebugSection("/proc/version", shellSnapshot.section("proc_version"))
        appendDebugSection("/proc/cmdline", shellSnapshot.section("proc_cmdline"))
        appendDebugSection("Selected getprop", shellSnapshot.section("getprop_selected"))
        appendDebugSection("Root manager", shellSnapshot.section("root_manager"))
        appendDebugSection("SELinux", shellSnapshot.section("selinux"))
    }

private fun buildBackendsText(
    context: Context,
    shellSnapshot: DebugShellSnapshot,
): String =
    buildString {
        appendDebugSection("Current boot_id", shellSnapshot.section("current_boot_id"))
        appendDebugSection("Backend evidence", buildBackendEvidence(shellSnapshot))
        appendDebugSection("kmod module.prop", shellSnapshot.section("kmod_prop"))
        appendDebugSection("kmod module state", shellSnapshot.section("kmod_module_state"))
        appendDebugSection("kmod live /proc/vpnhide_ctl", shellSnapshot.section("kmod_state"))
        appendDebugSection("kmod boot load_status", shellSnapshot.section("kmod_load_status"))
        appendDebugSection("kmod boot dmesg", shellSnapshot.section("kmod_load_dmesg"))
        appendDebugSection("kmod runtime targets", shellSnapshot.section("kmod_targets"))
        appendDebugSection("KPM module.prop", shellSnapshot.section("kpm_prop"))
        appendDebugSection("KPM module state", shellSnapshot.section("kpm_module_state"))
        appendDebugSection("KPM live activator state", shellSnapshot.section("kpm_state"))
        appendDebugSection("KPM boot load_status", shellSnapshot.section("kpm_load_status"))
        appendDebugSection("KPM runtime targets", shellSnapshot.section("kpm_targets"))
        appendDebugSection("KernelPatch runtime", shellSnapshot.section("kpatch_runtime"))
        appendDebugSection("Zygisk module.prop", shellSnapshot.section("zygisk_prop"))
        appendDebugSection("Zygisk module state", shellSnapshot.section("zygisk_module_state"))
        appendDebugSection("Zygisk heartbeat", shellSnapshot.section("zygisk_status"))
        appendDebugSection("Zygisk runtime modules", shellSnapshot.section("zygisk_runtime"))
        appendDebugSection("Zygisk runtime targets", shellSnapshot.section("zygisk_targets"))
        appendDebugSection("LSPosed hook state", shellSnapshot.section("lsposed_state"))
        appendDebugSection("LSPosed config DB", buildLsposedConfigText(context))
        appendDebugSection("LSPosed framework", shellSnapshot.section("lsposed_framework"))
        appendDebugSection("LSPosed files", shellSnapshot.section("lsposed_files"))
        appendDebugSection("Ports module.prop", shellSnapshot.section("ports_prop"))
        appendDebugSection("Ports module state", shellSnapshot.section("ports_module_state"))
        appendDebugSection("Ports load_status", shellSnapshot.section("ports_load_status"))
        appendDebugSection("Ports load_log", shellSnapshot.section("ports_load_log"))
        appendDebugSection("Ports observers", shellSnapshot.section("ports_observers"))
        appendDebugSection("Ports iptables state", shellSnapshot.section("ports_state"))
    }

private fun buildModulesText(shellSnapshot: DebugShellSnapshot): String =
    buildString {
        appendDebugSection("Module inventory", shellSnapshot.section("module_inventory"))
        appendDebugSection("Root manager", shellSnapshot.section("root_manager"))
        appendDebugSection("Kernel modules", shellSnapshot.section("proc_modules"))
    }

private fun buildConfigText(shellSnapshot: DebugShellSnapshot): String =
    buildString {
        appendDebugSection("canonical config", shellSnapshot.section("canonical_config"))
        appendDebugSection("hidden packages", shellSnapshot.section("hidden_pkgs"))
        appendDebugSection("observer UIDs", shellSnapshot.section("observer_uids"))
        appendDebugSection("kmod runtime targets", shellSnapshot.section("kmod_targets"))
        appendDebugSection("KPM runtime targets", shellSnapshot.section("kpm_targets"))
        appendDebugSection("Zygisk runtime targets", shellSnapshot.section("zygisk_targets"))
        appendDebugSection("Ports observers", shellSnapshot.section("ports_observers"))
        appendDebugSection("Ports load_status", shellSnapshot.section("ports_load_status"))
    }

private fun buildInterfacesText(shellSnapshot: DebugShellSnapshot): String =
    buildString {
        appendDebugSection("ip -d addr", shellSnapshot.section("network_addr"))
        appendDebugSection("Interface operstate", shellSnapshot.section("network_operstate"))
        appendDebugSection("ip route show table all", shellSnapshot.section("network_routes"))
        appendDebugSection("ip rule", shellSnapshot.section("network_rules"))
        appendDebugSection("Listening sockets", shellSnapshot.section("network_sockets"))
        appendDebugSection("dumpsys connectivity excerpt", shellSnapshot.section("connectivity_dump"))
    }

private fun buildProcNetText(shellSnapshot: DebugShellSnapshot): String =
    buildString {
        val sections =
            listOf(
                "route" to "proc_net_route",
                "ipv6_route" to "proc_net_ipv6_route",
                "if_inet6" to "proc_net_if_inet6",
                "tcp" to "proc_net_tcp",
                "tcp6" to "proc_net_tcp6",
                "udp" to "proc_net_udp",
                "udp6" to "proc_net_udp6",
                "dev" to "proc_net_dev",
                "fib_trie" to "proc_net_fib_trie",
            )
        for ((label, sectionName) in sections) {
            appendDebugSection("/proc/net/$label", shellSnapshot.section(sectionName))
        }
    }

private fun buildKernelText(shellSnapshot: DebugShellSnapshot): String =
    buildString {
        appendDebugSection("Kernel config", shellSnapshot.section("kernel_config"))
        appendDebugSection("Kernel modules", shellSnapshot.section("proc_modules"))
        appendDebugSection("Registered kprobes", shellSnapshot.section("kprobes"))
        appendDebugSection("Kernel symbols", shellSnapshot.section("kernel_symbols"))
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
        "VpnHide-LSPosed",
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

private fun DebugShellSnapshot.section(name: String): String = sections[name].orEmpty()

private fun StringBuilder.appendDebugSection(
    title: String,
    body: String,
) {
    appendLine("=== $title ===")
    appendLine(body.trimEnd().ifBlank { "(empty)" })
    appendLine()
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

private fun buildBackendEvidence(shellSnapshot: DebugShellSnapshot): String =
    buildString {
        fun evidence(
            label: String,
            section: String,
            predicate: (String) -> Boolean,
        ) {
            val raw = shellSnapshot.section(section)
            appendLine("$label: ${if (predicate(raw)) "present" else "not observed"}")
        }
        evidence("kmod live proc", "kmod_state") { it.contains("vpnhide 1 status") }
        evidence("KPM live ctl0", "kpm_state") { it.contains("vpnhide 1 status") }
        evidence("Zygisk heartbeat", "zygisk_status") { it.contains("boot_id=") }
        evidence("LSPosed hook state", "lsposed_state") { it.contains("vpnhide 1 status") }
        evidence("Ports activator status", "ports_load_status") { it.contains("loaded=1") }
        evidence("Ports iptables", "ports_state") { it.contains("vpnhide_out") || it.contains("vpnhide_out6") }
    }.trimEnd()

private fun buildLsposedConfigText(context: Context): String {
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

internal fun filterVpnHideDmesg(dmesg: String): String {
    val filtered =
        dmesg
            .lineSequence()
            .filter {
                it.contains("vpnhide", ignoreCase = true) ||
                    it.contains("kpm", ignoreCase = true) ||
                    it.contains("kretprobe", ignoreCase = true) ||
                    it.contains("[+] KP", ignoreCase = true)
            }.joinToString("\n")
    return filtered.ifBlank { "(no vpnhide dmesg entries)" }
}
