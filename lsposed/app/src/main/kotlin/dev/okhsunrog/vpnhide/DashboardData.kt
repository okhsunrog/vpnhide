package dev.okhsunrog.vpnhide

import android.content.pm.PackageManager
import android.database.sqlite.SQLiteDatabase
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import android.util.Log
import java.io.File

// ── Domain types — invalid states are unrepresentable ────────────────────

sealed interface ModuleState {
    data object NotInstalled : ModuleState

    data class Installed(
        val version: String?,
        val active: Boolean,
        val targetCount: Int,
    ) : ModuleState
}

sealed interface LsposedState {
    data object NotInstalled : LsposedState

    data class InstalledInactive(
        val version: String?,
    ) : LsposedState

    data class NeedsReboot(
        val version: String?,
    ) : LsposedState

    data class Active(
        val version: String?,
        val targetCount: Int,
    ) : LsposedState
}

sealed interface ProtectionCheck {
    data object NoVpn : ProtectionCheck

    data object NeedsRestart : ProtectionCheck

    data class Checked(
        val native: NativeResult,
        val java: JavaResult,
    ) : ProtectionCheck
}

sealed interface NativeResult {
    data object Ok : NativeResult

    data class Fail(
        val passed: Int,
        val failed: Int,
    ) : NativeResult

    data object NoModule : NativeResult
}

sealed interface JavaResult {
    data object Ok : JavaResult

    data class Fail(
        val failedChecks: Int,
    ) : JavaResult

    data object HooksInactive : JavaResult
}

private enum class NativeModuleKind { Kmod, Zygisk, Ports }

private sealed interface LsposedRuntime {
    data object Inactive : LsposedRuntime

    data class Active(
        val version: String?,
    ) : LsposedRuntime
}

private sealed interface LsposedFramework {
    data object NotInstalled : LsposedFramework

    data class Installed(
        val disabled: Boolean,
    ) : LsposedFramework
}

private sealed interface LsposedConfig {
    data object ModuleNotConfigured : LsposedConfig

    data object Disabled : LsposedConfig

    data class Enabled(
        val entries: List<String>,
        val hasSystemFramework: Boolean,
        val extraEntries: List<String>,
    ) : LsposedConfig
}

internal enum class IssueSeverity { ERROR, WARNING }

internal data class Issue(
    val severity: IssueSeverity,
    val text: String,
)

internal data class DashboardState(
    val kmod: ModuleState,
    val zygisk: ModuleState,
    val lsposed: LsposedState,
    val ports: ModuleState,
    val nativeInstallRecommendation: NativeInstallRecommendation?,
    val protection: ProtectionCheck,
    val issues: List<Issue>,
)

internal data class NativeInstallRecommendation(
    val androidVersion: String,
    val kernelVersion: String,
    val kernelBranch: String?,
    val recommendedArtifact: String,
    val preferKmod: Boolean,
)

private const val TAG = "VpnHide-Dashboard"

internal fun loadDashboardState(
    cm: ConnectivityManager,
    context: android.content.Context,
    selfNeedsRestart: Boolean,
): DashboardState {
    val issues = mutableListOf<Issue>()
    val res = context.resources
    val selfPkg = context.packageName

    fun err(text: String) {
        issues += Issue(IssueSeverity.ERROR, text)
    }

    fun warn(text: String) {
        issues += Issue(IssueSeverity.WARNING, text)
    }

    VpnHideLog.i(TAG, "=== Loading dashboard state ===")

    // ── Module detection ──
    // Strip the `v` prefix from module.prop versions at parse time so
    // everything downstream — dashboard rendering, issue text, update
    // checks — sees a plain semver string. APK versionName has no `v`
    // (Android convention); stamping `v` into module.prop follows the
    // Magisk convention but mixes badly when both show side by side.
    fun parseModuleProp(dir: String): Pair<Boolean, String?> {
        val (exitCode, out) = suExec("cat $dir/module.prop 2>/dev/null")
        if (exitCode != 0 || out.isBlank()) return false to null
        val version =
            out
                .lines()
                .firstOrNull { it.startsWith("version=") }
                ?.removePrefix("version=")
                ?.let(::normalizeVersion)
        return true to version
    }

    fun countTargets(path: String): Int {
        val (_, out) = suExec("cat $path 2>/dev/null || true")
        return out.lines().count { line ->
            val trimmed = line.trim()
            trimmed.isNotEmpty() && !trimmed.startsWith("#") && trimmed != selfPkg
        }
    }

    fun parseProps(raw: String): Map<String, String> =
        raw
            .lines()
            .mapNotNull {
                val parts = it.split("=", limit = 2)
                if (parts.size == 2) parts[0] to parts[1] else null
            }.toMap()

    fun buildModuleVersionIssue(
        kind: NativeModuleKind,
        moduleVersion: String,
        appVersion: String,
    ): String {
        val normalizedModuleVersion = normalizeVersion(moduleVersion)
        val normalizedAppVersion = normalizeVersion(appVersion)
        return when (compareSemver(normalizedModuleVersion, normalizedAppVersion)) {
            null, 0 -> {
                res.getString(
                    when (kind) {
                        NativeModuleKind.Kmod -> R.string.dashboard_issue_kmod_version_mismatch
                        NativeModuleKind.Zygisk -> R.string.dashboard_issue_zygisk_version_mismatch
                        NativeModuleKind.Ports -> R.string.dashboard_issue_ports_version_mismatch
                    },
                    moduleVersion,
                    appVersion,
                )
            }

            in Int.MIN_VALUE..-1 -> {
                res.getString(
                    when (kind) {
                        NativeModuleKind.Kmod -> R.string.dashboard_issue_update_kmod
                        NativeModuleKind.Zygisk -> R.string.dashboard_issue_update_zygisk
                        NativeModuleKind.Ports -> R.string.dashboard_issue_update_ports
                    },
                    moduleVersion,
                    appVersion,
                )
            }

            else -> {
                res.getString(
                    when (kind) {
                        NativeModuleKind.Kmod -> R.string.dashboard_issue_update_app_for_kmod
                        NativeModuleKind.Zygisk -> R.string.dashboard_issue_update_app_for_zygisk
                        NativeModuleKind.Ports -> R.string.dashboard_issue_update_app_for_ports
                    },
                    moduleVersion,
                    appVersion,
                )
            }
        }
    }

    fun androidMajorVersionLabel(): String {
        @Suppress("DEPRECATION")
        val release =
            if (Build.VERSION.SDK_INT >= 30) {
                Build.VERSION.RELEASE_OR_CODENAME
            } else {
                Build.VERSION.RELEASE
            }.substringBefore('.')
        return "Android $release"
    }

    fun parseKernelSeries(raw: String): String? = Regex("""\b(\d+\.\d+)""").find(raw)?.groupValues?.get(1)

    fun parseKernelAndroidBranch(raw: String): String? =
        Regex("""android(\d+)""")
            .find(raw)
            ?.groupValues
            ?.get(1)
            ?.let { "Android $it" }

    fun buildNativeInstallRecommendation(): NativeInstallRecommendation? {
        val (_, kernelRaw) = suExec("uname -r 2>/dev/null")
        val kernelVersion = kernelRaw.trim().ifBlank { return null }
        val kernelSeries = parseKernelSeries(kernelVersion)
        val kernelBranch = parseKernelAndroidBranch(kernelVersion)
        val artifactKeyVersion = kernelBranch ?: androidMajorVersionLabel()
        val supportedArtifact =
            when (artifactKeyVersion to kernelSeries) {
                "Android 12" to "5.10" -> "vpnhide-kmod-android12-5.10.zip"
                "Android 13" to "5.10" -> "vpnhide-kmod-android13-5.10.zip"
                "Android 13" to "5.15" -> "vpnhide-kmod-android13-5.15.zip"
                "Android 14" to "5.15" -> "vpnhide-kmod-android14-5.15.zip"
                "Android 14" to "6.1" -> "vpnhide-kmod-android14-6.1.zip"
                "Android 15" to "6.6" -> "vpnhide-kmod-android15-6.6.zip"
                "Android 16" to "6.12" -> "vpnhide-kmod-android16-6.12.zip"
                else -> null
            }

        return if (supportedArtifact != null) {
            NativeInstallRecommendation(
                androidVersion = artifactKeyVersion,
                kernelVersion = kernelVersion,
                kernelBranch = kernelBranch,
                recommendedArtifact = supportedArtifact,
                preferKmod = true,
            )
        } else {
            NativeInstallRecommendation(
                androidVersion = artifactKeyVersion,
                kernelVersion = kernelVersion,
                kernelBranch = kernelBranch,
                recommendedArtifact = "vpnhide-zygisk.zip",
                preferKmod = false,
            )
        }
    }

    fun resolveScopeEntryLabel(entry: String): String {
        if (entry == "system" || entry == "system/0") return "System Framework"

        val packageName = entry.substringBefore('/')
        val userId = entry.substringAfter('/', "")
        return try {
            val appInfo = context.packageManager.getApplicationInfo(packageName, 0)
            val appLabel =
                context.packageManager
                    .getApplicationLabel(appInfo)
                    .toString()
                    .trim()
            when {
                appLabel.isEmpty() -> packageName
                userId.isNotEmpty() && userId != "0" -> "$appLabel ($userId)"
                else -> appLabel
            }
        } catch (_: PackageManager.NameNotFoundException) {
            packageName
        }
    }

    fun readLsposedConfig(): LsposedConfig? {
        val dbCopy = File(context.cacheDir, "vpnhide_lspd_modules_config.db")
        val dbWalCopy = File(context.cacheDir, "vpnhide_lspd_modules_config.db-wal")
        val dbShmCopy = File(context.cacheDir, "vpnhide_lspd_modules_config.db-shm")
        dbCopy.delete()
        dbWalCopy.delete()
        dbShmCopy.delete()

        val dbPath = dbCopy.absolutePath
        val walPath = dbWalCopy.absolutePath
        val shmPath = dbShmCopy.absolutePath
        val sourceBase = "/data/adb/lspd/config/modules_config.db"
        val (copyExit, copyOut) =
            suExec(
                "cat $sourceBase > $dbPath && " +
                    "chmod 644 $dbPath && " +
                    "(cat $sourceBase-wal > $walPath 2>/dev/null && chmod 644 $walPath || true) && " +
                    "(cat $sourceBase-shm > $shmPath 2>/dev/null && chmod 644 $shmPath || true) && " +
                    "ls -l $dbPath $walPath $shmPath 2>/dev/null || true",
            )
        if (copyExit != 0 || !dbCopy.isFile) {
            VpnHideLog.w(TAG, "failed to copy LSPosed config db for inspection: exit=$copyExit out=$copyOut")
            return null
        }
        VpnHideLog.i(TAG, "lsposed db copy: ${copyOut.trim()}")

        return try {
            SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READONLY).use { db ->
                db
                    .rawQuery(
                        "SELECT mid, enabled FROM modules WHERE module_pkg_name = ?",
                        arrayOf(selfPkg),
                    ).use { moduleCursor ->
                        if (!moduleCursor.moveToFirst()) {
                            return LsposedConfig.ModuleNotConfigured
                        }

                        val mid = moduleCursor.getLong(0)
                        val enabled = moduleCursor.getInt(1) != 0
                        if (!enabled) {
                            return LsposedConfig.Disabled
                        }

                        val scopeEntries = mutableListOf<Pair<String, Int>>()
                        db
                            .rawQuery(
                                "SELECT app_pkg_name, user_id FROM scope WHERE mid = ? ORDER BY user_id, app_pkg_name",
                                arrayOf(mid.toString()),
                            ).use { scopeCursor ->
                                while (scopeCursor.moveToNext()) {
                                    scopeEntries += scopeCursor.getString(0) to scopeCursor.getInt(1)
                                }
                            }
                        val hasSystemFramework = scopeEntries.any { (pkg, userId) -> pkg == "system" && userId == 0 }
                        val renderedEntries =
                            scopeEntries.map { (pkg, userId) ->
                                if (pkg == "system" && userId == 0) {
                                    "system"
                                } else {
                                    "$pkg/$userId"
                                }
                            }
                        val extraEntries =
                            scopeEntries
                                .filterNot { (pkg, userId) ->
                                    (pkg == "system" && userId == 0) || pkg == selfPkg
                                }.map { (pkg, userId) -> "$pkg/$userId" }

                        LsposedConfig.Enabled(
                            entries = renderedEntries,
                            hasSystemFramework = hasSystemFramework,
                            extraEntries = extraEntries,
                        )
                    }
            }
        } catch (e: Exception) {
            VpnHideLog.w(TAG, "failed to inspect LSPosed config db: ${e.message}")
            null
        } finally {
            dbCopy.delete()
            dbWalCopy.delete()
            dbShmCopy.delete()
        }
    }

    fun detectLsposedFramework(): LsposedFramework {
        // Known module directory names for LSPosed / LSPosed-Next / Vector
        val knownIds = listOf("zygisk_vector", "zygisk_lsposed", "lsposed")
        val checkScript =
            knownIds
                .flatMap { id ->
                    listOf("/data/adb/modules/$id", "/data/adb/modules_update/$id")
                }.joinToString("; ") { dir ->
                    "if [ -f $dir/module.prop ]; then " +
                        "echo installed=1; " +
                        "echo disabled=\$([ -f $dir/disable ] && echo 1 || echo 0); " +
                        "exit 0; fi"
                } + "; echo installed=0"
        val (exitCode, out) = suExec(checkScript)
        val props = parseProps(out)
        val installed = exitCode == 0 && props["installed"] == "1"
        val disabled = props["disabled"] == "1"
        val framework =
            if (installed) {
                LsposedFramework.Installed(disabled = disabled)
            } else {
                LsposedFramework.NotInstalled
            }
        VpnHideLog.i(TAG, "lsposed framework: $framework (raw=$out)")
        return framework
    }

    // kmod
    val (kmodInstalled, kmodVersion) = parseModuleProp(KMOD_MODULE_DIR)
    val (_, procExists) = suExec("[ -f $PROC_TARGETS ] && echo 1 || echo 0")
    val kmodActive = kmodInstalled && procExists.trim() == "1"
    val kmodTargetCount = if (kmodInstalled) countTargets(KMOD_TARGETS) else 0
    val kmod: ModuleState =
        if (kmodInstalled) {
            ModuleState.Installed(kmodVersion, kmodActive, kmodTargetCount)
        } else {
            ModuleState.NotInstalled
        }
    VpnHideLog.i(TAG, "kmod: $kmod")

    // zygisk
    val (zygiskInstalled, zygiskVersion) = parseModuleProp(ZYGISK_MODULE_DIR)
    val zygiskStatusFile = File(context.filesDir, ZYGISK_STATUS_FILE_NAME)
    val zygiskStatusRaw =
        try {
            zygiskStatusFile.takeIf { it.isFile }?.readText().orEmpty()
        } catch (e: Exception) {
            VpnHideLog.w(TAG, "failed to read zygisk status heartbeat: ${e.message}")
            ""
        }
    val zygiskProps = parseProps(zygiskStatusRaw)
    val (_, currentBootId) = suExec("cat /proc/sys/kernel/random/boot_id 2>/dev/null")
    val zygiskBootId = zygiskProps["boot_id"]
    val zygiskActive = zygiskInstalled && zygiskBootId != null && zygiskBootId == currentBootId.trim()
    val zygiskTargetCount = if (zygiskInstalled) countTargets(ZYGISK_TARGETS) else 0
    val zygisk: ModuleState =
        if (zygiskInstalled) {
            ModuleState.Installed(zygiskVersion, zygiskActive, zygiskTargetCount)
        } else {
            ModuleState.NotInstalled
        }
    VpnHideLog.i(TAG, "zygisk: $zygisk (heartbeatBootId=$zygiskBootId currentBootId=${currentBootId.trim()})")

    // ports (iptables-based loopback blocker)
    val (portsInstalled, portsVersion) = parseModuleProp(PORTS_MODULE_DIR)
    val portsObserverCount =
        if (portsInstalled) countTargets(PORTS_OBSERVERS_FILE) else 0
    val (_, portsChainExists) = suExec("iptables -L vpnhide_out -n 2>/dev/null >/dev/null && echo 1 || echo 0")
    val portsActive = portsInstalled && portsChainExists.trim() == "1"
    val ports: ModuleState =
        if (portsInstalled) {
            ModuleState.Installed(portsVersion, portsActive, portsObserverCount)
        } else {
            ModuleState.NotInstalled
        }
    VpnHideLog.i(TAG, "ports: $ports")

    // Recommendation based purely on the kernel — used both by the install
    // card (only shown when nothing's installed) and by the "kmod-capable
    // kernel, only zygisk installed" warning (fires even after install).
    val kernelRecommendation = buildNativeInstallRecommendation()
    val nativeInstallRecommendation =
        if (kmod is ModuleState.NotInstalled && zygisk is ModuleState.NotInstalled) {
            kernelRecommendation
        } else {
            null
        }
    VpnHideLog.i(TAG, "nativeInstallRecommendation=$nativeInstallRecommendation kernelRec=$kernelRecommendation")

    // lsposed hook status
    val (_, hookStatusRaw) = suExec("cat ${HookEntry.HOOK_STATUS_FILE} 2>/dev/null || true")
    val hookProps = parseProps(hookStatusRaw)
    val hookVersion = hookProps["version"]
    val hookBootId = hookProps["boot_id"]
    val hooksActiveThisBoot = hookBootId != null && hookBootId == currentBootId.trim()
    val lsposedTargetCount = countTargets(LSPOSED_TARGETS)
    val lsposedFramework = detectLsposedFramework()
    val lsposedConfig =
        when (lsposedFramework) {
            LsposedFramework.NotInstalled -> {
                LsposedConfig.ModuleNotConfigured
            }

            is LsposedFramework.Installed -> {
                if (lsposedFramework.disabled) {
                    LsposedConfig.Disabled
                } else {
                    readLsposedConfig()
                }
            }
        }
    val lsposedRuntime: LsposedRuntime =
        if (hooksActiveThisBoot) {
            LsposedRuntime.Active(hookVersion)
        } else {
            LsposedRuntime.Inactive
        }

    val lsposed: LsposedState =
        when (lsposedRuntime) {
            is LsposedRuntime.Active -> {
                LsposedState.Active(lsposedRuntime.version, lsposedTargetCount)
            }

            LsposedRuntime.Inactive -> {
                when (lsposedConfig) {
                    null -> {
                        LsposedState.InstalledInactive(null)
                    }

                    LsposedConfig.ModuleNotConfigured -> {
                        when (lsposedFramework) {
                            LsposedFramework.NotInstalled -> LsposedState.NotInstalled
                            is LsposedFramework.Installed -> LsposedState.InstalledInactive(null)
                        }
                    }

                    LsposedConfig.Disabled -> {
                        LsposedState.InstalledInactive(null)
                    }

                    is LsposedConfig.Enabled -> {
                        if (lsposedConfig.hasSystemFramework) {
                            LsposedState.NeedsReboot(hookVersion)
                        } else {
                            LsposedState.InstalledInactive(null)
                        }
                    }
                }
            }
        }
    VpnHideLog.i(
        TAG,
        "lsposed: $lsposed (hookBootId=$hookBootId currentBootId=${currentBootId.trim()} framework=$lsposedFramework runtime=$lsposedRuntime config=$lsposedConfig)",
    )

    // ── Issues ──
    val hasNative = kmod is ModuleState.Installed || zygisk is ModuleState.Installed
    if (!hasNative) {
        err(res.getString(R.string.dashboard_issue_no_native))
    }
    if (lsposedFramework is LsposedFramework.NotInstalled && lsposed !is LsposedState.Active) {
        err(res.getString(R.string.dashboard_issue_lsposed_not_installed))
    }
    if (lsposed is LsposedState.NeedsReboot) {
        err(res.getString(R.string.dashboard_issue_reboot))
    }
    // Only report LSPosed config issues when hooks are not already active at runtime —
    // if hooks are active, the config is clearly working regardless of what we detect on disk
    if (lsposed !is LsposedState.Active) {
        when (lsposedConfig) {
            null -> {
                err(res.getString(R.string.dashboard_issue_lsposed_config_unreadable))
            }

            LsposedConfig.ModuleNotConfigured -> {
                if (lsposedFramework is LsposedFramework.Installed) {
                    err(res.getString(R.string.dashboard_issue_lsposed_not_enabled))
                }
            }

            LsposedConfig.Disabled -> {
                err(res.getString(R.string.dashboard_issue_lsposed_not_enabled))
            }

            is LsposedConfig.Enabled -> {
                if (!lsposedConfig.hasSystemFramework) {
                    err(res.getString(R.string.dashboard_issue_lsposed_no_system_scope))
                }
                if (lsposedConfig.extraEntries.isNotEmpty()) {
                    // Extra entries work, they're just cosmetic noise — warn.
                    warn(
                        res.getString(
                            R.string.dashboard_issue_lsposed_extra_scope,
                            lsposedConfig.extraEntries.map(::resolveScopeEntryLabel).joinToString(", "),
                        ),
                    )
                }
            }
        }
    }
    val appVersion = BuildConfig.VERSION_NAME
    // Version mismatches are warnings — modules keep working, user just needs to
    // update the lagging side. Full coverage is not affected by a patch-level gap.
    if (kmod is ModuleState.Installed && kmod.version != null && normalizeVersion(kmod.version) != normalizeVersion(appVersion)) {
        warn(buildModuleVersionIssue(NativeModuleKind.Kmod, kmod.version, appVersion))
    }
    if (zygisk is ModuleState.Installed && zygisk.version != null && normalizeVersion(zygisk.version) != normalizeVersion(appVersion)) {
        warn(buildModuleVersionIssue(NativeModuleKind.Zygisk, zygisk.version, appVersion))
    }
    if (ports is ModuleState.Installed && ports.version != null && normalizeVersion(ports.version) != normalizeVersion(appVersion)) {
        warn(buildModuleVersionIssue(NativeModuleKind.Ports, ports.version, appVersion))
    }
    val totalTargets = lsposedTargetCount + kmodTargetCount + zygiskTargetCount
    if (totalTargets == 0) {
        err(res.getString(R.string.dashboard_issue_no_targets))
    }
    if (ports is ModuleState.Installed && ports.targetCount == 0) {
        warn(res.getString(R.string.dashboard_issue_ports_no_observers))
    }
    if (lsposed is LsposedState.Active) {
        val runningVersion = lsposed.version
        if (runningVersion != null && runningVersion != appVersion) {
            VpnHideLog.w(TAG, "version mismatch: running=$runningVersion app=$appVersion")
            warn(res.getString(R.string.dashboard_issue_version_mismatch, runningVersion, appVersion))
        }
    }

    // ── Warnings: suboptimal-but-working setups ──

    // W1: kernel supports kmod, but user only installed zygisk. Zygisk is
    // in-process and theoretically detectable by anti-tamper; kmod is strictly
    // less fingerprinted when available.
    if (kernelRecommendation?.preferKmod == true &&
        zygisk is ModuleState.Installed &&
        kmod is ModuleState.NotInstalled
    ) {
        warn(
            res.getString(
                R.string.dashboard_issue_kmod_capable_but_zygisk,
                kernelRecommendation.recommendedArtifact,
            ),
        )
    }

    // W2: kmod and zygisk both active simultaneously — redundant native hooks,
    // larger fingerprint surface for anti-tamper SDKs (more hooked libc
    // entrypoints in /proc/self/maps). Zygisk is meant as a fallback only.
    if (kmod is ModuleState.Installed &&
        kmod.active &&
        zygisk is ModuleState.Installed &&
        zygisk.active
    ) {
        warn(res.getString(R.string.dashboard_issue_both_native_active))
    }

    // W3: user has debug logging turned on — VPN Hide is writing verbose lines
    // to logcat that a forensic reader with root can see. The flag file is
    // written by the Diagnostics → Debug logging toggle (separate PR); absent
    // file ⇒ default off ⇒ no warning.
    val (debugEnabledExit, debugEnabledRaw) = suExec("cat /data/system/vpnhide_debug_logging 2>/dev/null")
    if (debugEnabledExit == 0 && debugEnabledRaw.trim() == "1") {
        warn(res.getString(R.string.dashboard_issue_debug_logging_on))
    }

    // W4: SELinux Permissive exposes six detection vectors we rely on SELinux
    // to block (RTM_GETROUTE, /proc/net/{tcp,tcp6,udp,udp6,dev,fib_trie},
    // /sys/class/net). See the coverage table in the top-level README.
    val (_, getenforce) = suExec("getenforce 2>/dev/null")
    if (getenforce.trim().equals("Permissive", ignoreCase = true)) {
        warn(res.getString(R.string.dashboard_issue_selinux_permissive))
    }

    // ── Protection checks ──
    val vpnActive = isVpnActiveSync()
    VpnHideLog.i(TAG, "vpnActive=$vpnActive selfNeedsRestart=$selfNeedsRestart")

    val protection: ProtectionCheck =
        when {
            !vpnActive -> {
                ProtectionCheck.NoVpn
            }

            selfNeedsRestart -> {
                ProtectionCheck.NeedsRestart
            }

            else -> {
                val native =
                    if (hasNative) {
                        runNativeProtectionCheck()
                    } else {
                        NativeResult.NoModule
                    }
                VpnHideLog.i(TAG, "nativeResult=$native")

                val java =
                    if (lsposed is LsposedState.Active) {
                        runJavaProtectionCheck(cm)
                    } else {
                        JavaResult.HooksInactive
                    }
                VpnHideLog.i(TAG, "javaResult=$java")

                ProtectionCheck.Checked(native, java)
            }
        }

    VpnHideLog.i(TAG, "protection=$protection issues=$issues")
    VpnHideLog.i(TAG, "=== Dashboard state loaded ===")

    return DashboardState(
        kmod = kmod,
        zygisk = zygisk,
        lsposed = lsposed,
        ports = ports,
        nativeInstallRecommendation = nativeInstallRecommendation,
        protection = protection,
        issues = issues,
    )
}

private fun isVpnActiveSync(): Boolean {
    val (exitCode, output) = suExec("ls /sys/class/net/ 2>/dev/null")
    if (exitCode != 0) return false
    val vpnPrefixes = listOf("tun", "wg", "ppp", "tap", "ipsec", "xfrm")
    val vpnIfaces =
        output.lines().map { it.trim() }.filter { name ->
            name.isNotEmpty() && vpnPrefixes.any { name.startsWith(it) }
        }
    if (vpnIfaces.isEmpty()) {
        VpnHideLog.d(TAG, "isVpnActive: no VPN interfaces found")
        return false
    }
    return vpnIfaces.any { iface ->
        val (_, state) = suExec("cat /sys/class/net/$iface/operstate 2>/dev/null")
        val up = state.trim() == "unknown" || state.trim() == "up"
        VpnHideLog.d(TAG, "isVpnActive: $iface operstate=${state.trim()} up=$up")
        up
    }
}

private fun runNativeProtectionCheck(): NativeResult {
    val checks =
        listOf(
            "ioctl_flags" to { NativeChecks.checkIoctlSiocgifflags() },
            "ioctl_mtu" to { NativeChecks.checkIoctlSiocgifmtu() },
            "ioctl_conf" to { NativeChecks.checkIoctlSiocgifconf() },
            "getifaddrs" to { NativeChecks.checkGetifaddrs() },
            "netlink_getlink" to { NativeChecks.checkNetlinkGetlink() },
            "netlink_getlink_recv" to { NativeChecks.checkNetlinkGetlinkRecv() },
            "netlink_getroute" to { NativeChecks.checkNetlinkGetroute() },
            "proc_route" to { NativeChecks.checkProcNetRoute() },
            "proc_ipv6_route" to { NativeChecks.checkProcNetIpv6Route() },
            "proc_if_inet6" to { NativeChecks.checkProcNetIfInet6() },
            "proc_tcp" to { NativeChecks.checkProcNetTcp() },
            "proc_tcp6" to { NativeChecks.checkProcNetTcp6() },
            "proc_udp" to { NativeChecks.checkProcNetUdp() },
            "proc_udp6" to { NativeChecks.checkProcNetUdp6() },
            "proc_dev" to { NativeChecks.checkProcNetDev() },
            "proc_fib_trie" to { NativeChecks.checkProcNetFibTrie() },
            "sys_class_net" to { NativeChecks.checkSysClassNet() },
        )

    var passed = 0
    var failed = 0
    var skipped = 0
    for ((name, check) in checks) {
        try {
            val result = check()
            when {
                result.startsWith("NETWORK_BLOCKED:") -> {
                    skipped++
                    VpnHideLog.d(TAG, "native[$name]: NETWORK_BLOCKED")
                }

                result.contains("SELinux") ||
                    result.contains("EACCES") ||
                    result.contains("Permission denied") -> {
                    skipped++
                    VpnHideLog.d(TAG, "native[$name]: SELinux blocked, skipping")
                }

                result.startsWith("PASS") -> {
                    passed++
                    VpnHideLog.d(TAG, "native[$name]: PASS")
                }

                else -> {
                    failed++
                    VpnHideLog.w(TAG, "native[$name]: FAIL — $result")
                }
            }
        } catch (e: Exception) {
            failed++
            Log.e(TAG, "native[$name]: exception — ${e.message}")
        }
    }

    VpnHideLog.i(TAG, "native protection: passed=$passed failed=$failed skipped=$skipped")
    return when {
        passed == 0 && failed == 0 -> NativeResult.Ok

        // all SELinux-blocked = nothing leaked
        failed == 0 -> NativeResult.Ok

        passed > 0 -> NativeResult.Fail(passed, failed)

        else -> NativeResult.Fail(0, failed)
    }
}

@Suppress("DEPRECATION")
private fun runJavaProtectionCheck(cm: ConnectivityManager): JavaResult {
    val net = cm.activeNetwork
    if (net == null) {
        VpnHideLog.d(TAG, "java: no active network")
        return JavaResult.Ok
    }
    val caps = cm.getNetworkCapabilities(net)
    if (caps == null) {
        VpnHideLog.d(TAG, "java: no capabilities")
        return JavaResult.Ok
    }

    var failed = 0

    val hasVpn = caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
    if (hasVpn) failed++
    VpnHideLog.d(TAG, "java: hasTransport(VPN)=$hasVpn")

    val notVpn = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
    if (!notVpn) failed++
    VpnHideLog.d(TAG, "java: hasCapability(NOT_VPN)=$notVpn")

    // NetworkCapabilities.getTransportInfo() was introduced in API 29;
    // on API 28 (Android 9) the leak path does not exist, so skip the check.
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        val info = caps.transportInfo
        val isVpnTi = info?.javaClass?.name?.contains("VpnTransportInfo") == true
        if (isVpnTi) failed++
        VpnHideLog.d(TAG, "java: transportInfo=${info?.javaClass?.name} isVpn=$isVpnTi")
    } else {
        VpnHideLog.d(TAG, "java: transportInfo check skipped (API < 29)")
    }

    val vpnNets =
        cm.allNetworks.count {
            cm.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true
        }
    if (vpnNets > 0) failed++
    VpnHideLog.d(TAG, "java: allNetworks vpnCount=$vpnNets")

    val lp = cm.getLinkProperties(net)
    val ifname = lp?.interfaceName
    val vpnPrefixes = listOf("tun", "wg", "ppp", "tap", "ipsec", "xfrm")
    val vpnIfname = ifname != null && vpnPrefixes.any { ifname.startsWith(it) }
    if (vpnIfname) failed++
    VpnHideLog.d(TAG, "java: linkProperties ifname=$ifname isVpn=$vpnIfname")

    VpnHideLog.i(TAG, "java protection: failed=$failed")
    return if (failed == 0) JavaResult.Ok else JavaResult.Fail(failed)
}
