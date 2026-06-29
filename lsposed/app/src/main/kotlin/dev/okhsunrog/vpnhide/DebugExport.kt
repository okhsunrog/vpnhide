package dev.okhsunrog.vpnhide

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.Uri
import android.os.Build
import android.util.Log
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.FiberManualRecord
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.generated.IfaceLists
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard
import dev.okhsunrog.vpnhide.ui.components.GroupedCard
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.BufferedReader
import java.io.File
import java.io.InputStreamReader
import java.net.NetworkInterface
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

private const val TAG = "VPNHideTest"

// ==========================================================================
//  Debug log export
// ==========================================================================

internal suspend fun exportDebugZip(
    cm: ConnectivityManager,
    context: android.content.Context,
    selfNeedsRestart: Boolean,
): File? =
    withContext(Dispatchers.IO) {
        // Force-enable debug logging across all four sinks (app, system_server,
        // zygisk, kmod) while the capture runs so the dump contains VpnHide-
        // tagged lines + verbose dmesg even when the user's persistent toggle
        // is OFF (the default). We restore to whatever the SharedPreferences
        // say at the end — if the user happens to flip the UI toggle mid-
        // capture, we honor their final choice instead of blindly rolling
        // back. applyDebugLoggingRuntime drives all four sinks uniformly, so
        // there's no ad-hoc /proc/vpnhide_debug flip here anymore.
        val loggingWasForced = !VpnHideLog.enabled
        if (loggingWasForced) applyDebugLoggingRuntime(true)
        try {
            // 1. Clear dmesg so we only capture fresh output from the
            //    kmod hooks fired by runAllChecks below.
            suExec("dmesg -c > /dev/null 2>&1")

            // 2. Run all diagnostic checks (this triggers kmod hooks)
            val checkResults = runAllChecks(cm, context)

            // 3. Capture dmesg right after checks
            val (_, dmesg) = suExec("dmesg 2>/dev/null")

            // 4. Collect everything into named files — each section is its own
            //    builder below.
            val files =
                mapOf(
                    "dmesg_vpnhide.txt" to dmesg.lines().filter { it.contains("vpnhide") }.joinToString("\n"),
                    "dmesg_full.txt" to dmesg,
                    "diagnostics.txt" to buildDiagnosticsText(checkResults),
                    "device_info.txt" to buildDeviceInfoText(context, selfNeedsRestart),
                    "modules.txt" to buildModuleInfoText(),
                    "config.txt" to buildTargetsText(),
                    "interfaces.txt" to buildInterfacesText(),
                    "proc_net.txt" to buildProcNetText(),
                    "logcat.txt" to captureDebugLogcat().ifEmpty { "(no logcat entries)" },
                )

            // Create zip
            val timestamp = SimpleDateFormat("yyyyMMdd_HHmmss", Locale.US).format(Date())
            val zipFile = File(context.cacheDir, "vpnhide_debug_$timestamp.zip")
            ZipOutputStream(zipFile.outputStream()).use { zos ->
                for ((name, content) in files) {
                    zos.putNextEntry(ZipEntry(name))
                    zos.write(content.toByteArray())
                    zos.closeEntry()
                }
            }
            zipFile
        } catch (e: Exception) {
            Log.e(TAG, "Debug export failed", e)
            null
        } finally {
            if (loggingWasForced) {
                val target = isEnabledInPrefs(context)
                if (VpnHideLog.enabled != target) applyDebugLoggingRuntime(target)
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
    context: android.content.Context,
    selfNeedsRestart: Boolean,
): String {
    val (_, kernelVersion) = suExec("uname -r 2>/dev/null")
    val (_, procVersion) = suExec("cat /proc/version 2>/dev/null")
    val (_, selinuxMode) = suExec("getenforce 2>/dev/null")
    return buildString {
        appendLine("Device: ${Build.MANUFACTURER} ${Build.MODEL}")
        appendLine("Android: ${Build.VERSION.RELEASE} (SDK ${Build.VERSION.SDK_INT})")
        appendLine("Kernel: ${kernelVersion.trim()}")
        appendLine("Kernel full: ${procVersion.trim()}")
        appendLine("ABI: ${Build.SUPPORTED_ABIS.joinToString()}")
        appendLine("SELinux: ${selinuxMode.trim()}")
        appendLine("App package: ${context.packageName}")
        try {
            val pInfo = context.packageManager.getPackageInfo(context.packageName, 0)
            appendLine("App version: ${pInfo.versionName}")
        } catch (_: Exception) {
        }
        appendLine("selfNeedsRestart: $selfNeedsRestart")
        appendLine()
        appendLine("=== Root manager ===")
        val (_, magiskVer) = suExec("magisk -V 2>/dev/null")
        val (_, magiskVerName) = suExec("magisk -v 2>/dev/null")
        if (magiskVer.isNotBlank()) {
            appendLine("Magisk: ${magiskVerName.trim()} (${magiskVer.trim()})")
        }
        val (_, ksuVer) = suExec("cat /data/adb/ksu/version 2>/dev/null")
        if (ksuVer.isNotBlank()) {
            appendLine("KernelSU: ${ksuVer.trim()}")
        }
        val (exitKsuNext, ksuNextVer) = suExec("ksud --version 2>/dev/null")
        if (exitKsuNext == 0 && ksuNextVer.isNotBlank()) {
            appendLine("KernelSU-Next: ${ksuNextVer.trim()}")
        }
        if (magiskVer.isBlank() && ksuVer.isBlank() && (exitKsuNext != 0 || ksuNextVer.isBlank())) {
            appendLine("(unknown root manager)")
        }
    }
}

private fun buildModuleInfoText(): String =
    buildString {
        appendLine("=== Kernel module (kmod) ===")
        val (_, kmodProp) = suExec("cat /data/adb/modules/vpnhide_kmod/module.prop 2>/dev/null")
        appendLine(kmodProp.ifEmpty { "Not installed" })
        appendLine()
        appendLine("=== kmod load_status (boot-time diagnostics) ===")
        val (_, loadStatus) = suExec("cat $KMOD_LOAD_STATUS_FILE 2>/dev/null")
        appendLine(loadStatus.ifEmpty { "(not available — module never ran post-fs-data.sh this boot)" })
        appendLine()
        appendLine("=== Current boot_id ===")
        val (_, curBootId) = suExec("cat /proc/sys/kernel/random/boot_id 2>/dev/null")
        appendLine(curBootId.trim().ifEmpty { "(not available)" })
        appendLine()
        appendLine("=== kmod load_dmesg ===")
        val (_, loadDmesg) = suExec("cat $KMOD_LOAD_DMESG_FILE 2>/dev/null")
        appendLine(loadDmesg.ifEmpty { "(not captured)" })
        appendLine()
        appendLine("=== Zygisk module ===")
        val (_, zygiskProp) = suExec("cat /data/adb/modules/vpnhide_zygisk/module.prop 2>/dev/null")
        appendLine(zygiskProp.ifEmpty { "Not installed" })
        appendLine()
        appendLine("=== Registered kretprobes ===")
        val (_, kprobes) = suExec("cat /sys/kernel/debug/kprobes/list 2>/dev/null | grep vpnhide")
        appendLine(kprobes.ifEmpty { "(not available or no vpnhide probes)" })
        appendLine()
        appendLine("=== Kernel symbols (hooked functions) ===")
        val symbols =
            listOf("dev_ioctl", "dev_ifconf", "rtnl_fill_ifinfo", "inet6_fill_ifaddr", "inet_fill_ifaddr", "fib_route_seq_show")
        for (sym in symbols) {
            val (_, line) = suExec("cat /proc/kallsyms 2>/dev/null | grep -w $sym | head -3")
            appendLine("$sym: ${line.trim().ifEmpty { "(not found)" }}")
        }
        appendLine()
        appendLine("=== LSPosed configuration ===")
        val (_, lsposedDb) =
            suExec(
                "sqlite3 /data/adb/lspd/config/modules_config.db " +
                    "\"SELECT mid, module_pkg_name, enabled FROM modules WHERE module_pkg_name LIKE '%vpnhide%';\" 2>/dev/null",
            )
        appendLine(lsposedDb.ifEmpty { "(not available or module not in LSPosed)" })
        val (_, lsposedScope) =
            suExec(
                "sqlite3 /data/adb/lspd/config/modules_config.db " +
                    "\"SELECT s.app_pkg_name FROM scope s JOIN modules m ON s.mid=m.mid WHERE m.module_pkg_name LIKE '%vpnhide%';\" 2>/dev/null",
            )
        if (lsposedScope.isNotBlank()) {
            appendLine("Scope: ${lsposedScope.trim()}")
        }
    }

private fun buildTargetsText(): String =
    buildString {
        appendLine("=== /proc/vpnhide_ctl (live status + stats) ===")
        appendLine(suExec("cat $PROC_CTL 2>/dev/null").second.ifEmpty { "(empty)" })
        appendLine()
        appendLine("=== LSPosed state (live status + stats) ===")
        appendLine(suExec("cat $LSPOSED_STATE_FILE 2>/dev/null").second.ifEmpty { "(empty)" })
        appendLine()
        appendLine("=== canonical config ===")
        appendLine(suExec("cat $CANONICAL_CONFIG_FILE 2>/dev/null").second.ifEmpty { "(empty)" })
    }

private fun buildInterfacesText(): String =
    buildString {
        appendLine("=== ip -d addr ===")
        appendLine(suExec("ip -d addr 2>/dev/null").second.ifEmpty { "(not available)" })
        appendLine()
        appendLine("=== Interface operstate ===")
        val (_, operstate) =
            suExec(
                "for iface in /sys/class/net/*; do " +
                    "echo \"\$(basename \$iface): \$(cat \$iface/operstate 2>/dev/null)\"; " +
                    "done",
            )
        appendLine(operstate.ifEmpty { "(not available)" })
        appendLine()
        appendLine("=== ip route show table all ===")
        appendLine(suExec("ip route show table all 2>/dev/null").second.ifEmpty { "(not available)" })
        appendLine()
        appendLine("=== ip rule ===")
        appendLine(suExec("ip rule 2>/dev/null").second.ifEmpty { "(not available)" })
    }

private fun buildProcNetText(): String =
    buildString {
        for (pf in listOf("route", "ipv6_route", "if_inet6", "tcp", "tcp6", "udp", "udp6", "dev")) {
            appendLine("=== /proc/net/$pf ===")
            appendLine(suExec("cat /proc/net/$pf 2>/dev/null").second.ifEmpty { "(not available)" })
            appendLine()
        }
    }

private fun captureDebugLogcat(): String {
    val tags =
        listOf(
            "VPNHideTest:*",
            "VpnHide:*",
            "VpnHide-Dashboard:*",
            "VpnHide-LSPosed:*",
            // zygisk's android_logger uses this tag (see zygisk/src/lib.rs:LOG_TAG);
            // without it the export is missing all native-side hook logs.
            "vpnhide-zygisk:*",
        ).joinToString(" ")
    val (exit, output) = suExec("logcat -d -b all -v threadtime -s $tags 2>/dev/null")
    return if (exit == 0) output else "(logcat failed: exit=$exit)\n$output"
}
