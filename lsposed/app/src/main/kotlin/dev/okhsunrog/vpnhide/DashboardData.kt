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
        // Only populated for kmod builds that carry the stamped `gkiVariant=` field
        // in module.prop (CI-built zips from v0.6.3+). Older builds report null.
        val gkiVariant: String? = null,
        // Non-null when the module is installed but the installation itself
        // is permanently broken (distinct from "active=false" which usually
        // just means a reboot is pending). UI colors the card red.
        val brokenReason: KmodBrokenReason? = null,
    ) : ModuleState
}

enum class KmodBrokenReason {
    WrongVariant,
    UnsupportedKernel,
    MissingKprobes,
    UnknownVariantInactive,
    AmbiguousLoadFailed,
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

internal enum class NativeModuleKind { Kmod, Zygisk, Ports }

internal data class ModuleMismatch(
    val kind: NativeModuleKind,
    val moduleVersion: String,
    val appVersion: String,
)

// Pure: given a list of (state, kind) pairs and the app version, returns
// the subset whose base version disagrees with the app. Extracted so the
// three kmod / zygisk / ports callsites in loadDashboardState share one
// code path instead of three near-identical if-blocks.
internal fun detectModuleMismatches(
    modules: List<Pair<ModuleState, NativeModuleKind>>,
    appVersion: String,
): List<ModuleMismatch> =
    modules.mapNotNull { (state, kind) ->
        val installed = state as? ModuleState.Installed ?: return@mapNotNull null
        val moduleVersion = installed.version ?: return@mapNotNull null
        if (versionsMismatch(moduleVersion, appVersion)) {
            ModuleMismatch(kind, moduleVersion, appVersion)
        } else {
            null
        }
    }

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
    val kmodLoadStatus: KmodLoadStatus?,
    val protection: ProtectionCheck,
    val issues: List<Issue>,
)

internal data class NativeInstallRecommendation(
    val androidVersion: String,
    val kernelVersion: String,
    val kernelBranch: String?,
    val recommendedArtifact: String,
    val recommendedGkiVariant: String?,
    val preferKmod: Boolean,
    // Set when the kernel's GKI KMI couldn't be parsed from uname -r but the
    // kernel series ships with multiple KMI variants (5.10: android12 / 13;
    // 5.15: android13 / 14). Both candidates are valid picks — the UI shows
    // the primary plus "if it doesn't load, try the alternative". Series with
    // a single shipping variant (6.1 / 6.6 / 6.12) stay unambiguous even
    // without a KMI tag.
    val variantAmbiguous: Boolean = false,
    val alternativeArtifact: String? = null,
    val alternativeGkiVariant: String? = null,
)

// Boot-time diagnostics written by kmod/module/post-fs-data.sh into
// /data/adb/vpnhide_kmod/load_status. Stays valid across reboots,
// so bootId is compared against the current boot to know if the
// record is fresh.
internal data class KmodLoadStatus(
    val timestamp: Long?,
    val bootId: String?,
    val unameR: String?,
    val gkiVariant: String?,
    val kmodVersion: String?,
    val rootManager: String?,
    val kprobes: String?,
    val kretprobes: String?,
    val insmodExit: Int?,
    val loaded: Boolean,
    val insmodStderr: String?,
    val dmesgTail: String?,
    val freshForCurrentBoot: Boolean,
)

private const val TAG = "VpnHide-Dashboard"

internal fun parseKernelSeries(raw: String): String? = Regex("""\b(\d+\.\d+)""").find(raw)?.groupValues?.get(1)

internal fun parseKernelAndroidBranch(raw: String): String? =
    Regex("""android(\d+)""")
        .find(raw)
        ?.groupValues
        ?.get(1)
        ?.let { "Android $it" }

/**
 * Pick the right native-module artifact for the device based on its
 * kernel version (from `uname -r`) and Android OS label (from
 * `Build.VERSION.RELEASE`). Pulled out as a pure top-level function
 * so it can be unit-tested without a real device — the `uname -r`
 * read and `Build.VERSION` probe happen in the caller.
 *
 * Strategy, in order:
 *  1. Exact `(GKI KMI × kernel series)` match from the supported
 *     shipping matrix → specific kmod zip, preferKmod=true.
 *  2. KMI tag missing from `uname -r` (custom kernel stripped it)
 *     but the kernel series is GKI-shipping:
 *       - 6.1 / 6.6 / 6.12 have a single shipping variant each →
 *         deterministic kmod recommendation, preferKmod=true.
 *       - 5.10 / 5.15 have two shipping variants each → return the
 *         primary plus an alternative via `variantAmbiguous=true`;
 *         the UI shows "try primary, if it doesn't load try alt".
 *  3. Pre-GKI series (<5.10) or unparseable kernel version → fall
 *     back to zygisk (preferKmod=false) since we have no kmod
 *     binaries that can load against such kernels' Module.symvers.
 *
 * Returns `null` only if [kernelRaw] is blank (no uname output).
 * `deviceAndroidLabel` is only reflected back in the returned
 * `androidVersion` for display — it's never used for KMI matching
 * (those spaces are independent: an Android 15 ROM routinely runs
 * an android12 KMI kernel).
 */
internal fun buildNativeInstallRecommendation(
    kernelRaw: String,
    deviceAndroidLabel: String,
): NativeInstallRecommendation? {
    val kernelVersion = kernelRaw.trim().ifBlank { return null }
    val kernelSeries = parseKernelSeries(kernelVersion)
    val kernelBranch = parseKernelAndroidBranch(kernelVersion) // GKI KMI

    data class KmiMatch(
        val kmi: String,
        val zip: String,
    )

    val exact: KmiMatch? =
        when (kernelBranch to kernelSeries) {
            "Android 12" to "5.10" -> KmiMatch("android12-5.10", "vpnhide-kmod-android12-5.10.zip")
            "Android 13" to "5.10" -> KmiMatch("android13-5.10", "vpnhide-kmod-android13-5.10.zip")
            "Android 13" to "5.15" -> KmiMatch("android13-5.15", "vpnhide-kmod-android13-5.15.zip")
            "Android 14" to "5.15" -> KmiMatch("android14-5.15", "vpnhide-kmod-android14-5.15.zip")
            "Android 14" to "6.1" -> KmiMatch("android14-6.1", "vpnhide-kmod-android14-6.1.zip")
            "Android 15" to "6.6" -> KmiMatch("android15-6.6", "vpnhide-kmod-android15-6.6.zip")
            "Android 16" to "6.12" -> KmiMatch("android16-6.12", "vpnhide-kmod-android16-6.12.zip")
            else -> null
        }
    if (exact != null) {
        return NativeInstallRecommendation(
            androidVersion = deviceAndroidLabel,
            kernelVersion = kernelVersion,
            kernelBranch = kernelBranch,
            recommendedArtifact = exact.zip,
            recommendedGkiVariant = exact.kmi,
            preferKmod = true,
        )
    }

    val fallback: Pair<KmiMatch, KmiMatch?>? =
        when (kernelSeries) {
            "5.10" -> {
                KmiMatch("android12-5.10", "vpnhide-kmod-android12-5.10.zip") to
                    KmiMatch("android13-5.10", "vpnhide-kmod-android13-5.10.zip")
            }

            "5.15" -> {
                KmiMatch("android13-5.15", "vpnhide-kmod-android13-5.15.zip") to
                    KmiMatch("android14-5.15", "vpnhide-kmod-android14-5.15.zip")
            }

            "6.1" -> {
                KmiMatch("android14-6.1", "vpnhide-kmod-android14-6.1.zip") to null
            }

            "6.6" -> {
                KmiMatch("android15-6.6", "vpnhide-kmod-android15-6.6.zip") to null
            }

            "6.12" -> {
                KmiMatch("android16-6.12", "vpnhide-kmod-android16-6.12.zip") to null
            }

            else -> {
                null
            }
        }
    if (fallback != null) {
        val (primary, alternative) = fallback
        return NativeInstallRecommendation(
            androidVersion = deviceAndroidLabel,
            kernelVersion = kernelVersion,
            kernelBranch = kernelBranch,
            recommendedArtifact = primary.zip,
            recommendedGkiVariant = primary.kmi,
            preferKmod = true,
            variantAmbiguous = alternative != null,
            alternativeArtifact = alternative?.zip,
            alternativeGkiVariant = alternative?.kmi,
        )
    }

    return NativeInstallRecommendation(
        androidVersion = deviceAndroidLabel,
        kernelVersion = kernelVersion,
        kernelBranch = kernelBranch,
        recommendedArtifact = "vpnhide-zygisk.zip",
        recommendedGkiVariant = null,
        preferKmod = false,
    )
}

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
    data class ModulePropInfo(
        val installed: Boolean,
        val version: String?,
        val gkiVariant: String?,
    )

    // Older CI-built zips (between commit 3fc7355 "don't dirty committed
    // module.prop when injecting updateJson" and the gkiVariant stamping)
    // didn't stamp `gkiVariant=` but their injected updateJson URL already
    // encodes the KMI: `.../update-kmod-<kmi>.json`. Recover the variant
    // from there so wrong-variant detection works for existing installs
    // without requiring a reinstall.
    val updateJsonKmiRegex = Regex("""update-kmod-([^/]+)\.json""")

    fun parseModuleProp(dir: String): ModulePropInfo {
        val (exitCode, out) = suExec("cat $dir/module.prop 2>/dev/null")
        if (exitCode != 0 || out.isBlank()) return ModulePropInfo(false, null, null)
        var version: String? = null
        var gkiVariant: String? = null
        var updateJsonKmi: String? = null
        for (line in out.lines()) {
            when {
                line.startsWith("version=") -> {
                    version = normalizeVersion(line.removePrefix("version="))
                }

                line.startsWith("gkiVariant=") -> {
                    gkiVariant = line.removePrefix("gkiVariant=").trim().ifBlank { null }
                }

                line.startsWith("updateJson=") -> {
                    updateJsonKmi =
                        updateJsonKmiRegex
                            .find(line.removePrefix("updateJson="))
                            ?.groupValues
                            ?.get(1)
                }
            }
        }
        return ModulePropInfo(true, version, gkiVariant ?: updateJsonKmi)
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

    fun readKmodLoadStatus(currentBootId: String): KmodLoadStatus? {
        val (exit, raw) = suExec("cat $KMOD_LOAD_STATUS_FILE 2>/dev/null")
        if (exit != 0 || raw.isBlank()) return null
        val props = parseProps(raw)
        // load_dmesg is a separate file because its content spans many
        // lines and the key=value format can't carry them.
        val (_, dmesgRaw) = suExec("cat $KMOD_LOAD_DMESG_FILE 2>/dev/null")
        val bootId = props["boot_id"]?.trim()
        return KmodLoadStatus(
            timestamp = props["timestamp"]?.trim()?.toLongOrNull(),
            bootId = bootId,
            unameR = props["uname_r"]?.trim(),
            gkiVariant = props["gki_variant"]?.trim()?.ifBlank { null },
            kmodVersion = props["kmod_version"]?.trim()?.ifBlank { null },
            rootManager = props["root_manager"]?.trim()?.ifBlank { null },
            kprobes = props["kprobes"]?.trim()?.ifBlank { null },
            kretprobes = props["kretprobes"]?.trim()?.ifBlank { null },
            insmodExit = props["insmod_exit"]?.trim()?.toIntOrNull(),
            loaded = props["loaded"]?.trim() == "1",
            insmodStderr = props["insmod_stderr"]?.trim()?.ifBlank { null },
            dmesgTail = dmesgRaw.trim().ifBlank { null },
            freshForCurrentBoot = bootId != null && bootId == currentBootId,
        )
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
    val kmodProp = parseModuleProp(KMOD_MODULE_DIR)
    val (_, procExists) = suExec("[ -f $PROC_TARGETS ] && echo 1 || echo 0")
    val kmodActive = kmodProp.installed && procExists.trim() == "1"
    val kmodTargetCount = if (kmodProp.installed) countTargets(KMOD_TARGETS) else 0
    // Built without brokenReason — populated below after kernelRecommendation
    // and kmodLoadStatus are ready.
    val kmodRaw: ModuleState =
        if (kmodProp.installed) {
            ModuleState.Installed(
                version = kmodProp.version,
                active = kmodActive,
                targetCount = kmodTargetCount,
                gkiVariant = kmodProp.gkiVariant,
            )
        } else {
            ModuleState.NotInstalled
        }
    VpnHideLog.i(TAG, "kmodRaw: $kmodRaw")

    // zygisk
    val zygiskProp = parseModuleProp(ZYGISK_MODULE_DIR)
    val zygiskInstalled = zygiskProp.installed
    val zygiskVersion = zygiskProp.version
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
    val portsProp = parseModuleProp(PORTS_MODULE_DIR)
    val portsObserverCount =
        if (portsProp.installed) countTargets(PORTS_OBSERVERS_FILE) else 0
    val (_, portsChainExists) = suExec("iptables -L vpnhide_out -n 2>/dev/null >/dev/null && echo 1 || echo 0")
    val portsActive = portsProp.installed && portsChainExists.trim() == "1"
    val ports: ModuleState =
        if (portsProp.installed) {
            ModuleState.Installed(portsProp.version, portsActive, portsObserverCount)
        } else {
            ModuleState.NotInstalled
        }
    VpnHideLog.i(TAG, "ports: $ports")

    // Recommendation based purely on the kernel — used by the install card,
    // the "kmod-capable kernel, only zygisk installed" warning (W1), and the
    // wrong-variant detection below.
    val (_, kernelRaw) = suExec("uname -r 2>/dev/null")
    val kernelRecommendation = buildNativeInstallRecommendation(kernelRaw, androidMajorVersionLabel())
    val kmodLoadStatus = readKmodLoadStatus(currentBootId.trim())
    VpnHideLog.i(TAG, "kmodLoadStatus=$kmodLoadStatus")

    // Decide whether to surface the install-recommendation card.
    // Show when:
    //  - neither native module installed (classic first-install flow), or
    //  - kmod installed but its stamped gkiVariant doesn't match what the
    //    device needs (wrong zip — user needs to reinstall the correct one), or
    //  - kmod installed without gkiVariant (old build) AND not loaded —
    //    variant unknown + broken is almost always a variant mismatch, or
    //  - kmod installed on a kernel with no matching vpnhide-kmod variant at
    //    all (non-GKI / unsupported combo) — we recommend zygisk instead so
    //    the user doesn't wait for the kmod to "just work".
    val recommendedKmi = kernelRecommendation?.recommendedGkiVariant
    // Two cross-cutting gates on kmod warnings:
    //   !kmodRaw.active — an active kmod (/proc/vpnhide_targets present)
    //     is empirical proof the installation works, so a heuristic
    //     saying otherwise is wrong. Applied to every warning below.
    //   kmodLoadStatus?.freshForCurrentBoot == true (AmbiguousLoadFailed
    //     only) — that warning is specifically about "the module we
    //     installed tried to insmod this boot and failed, pick the
    //     other candidate", so it only makes sense once post-fs-data
    //     has actually attempted a load. The other warnings are
    //     deterministic from the variant stamp / kernel-series tables
    //     and are valid even before the first post-install boot.
    // For an ambiguous recommendation (variantAmbiguous=true), either
    // candidate is a valid install, so kmodVariantMismatch must check
    // both recommendedGkiVariant AND alternativeGkiVariant before
    // deciding it's a real mismatch.
    val kmodVariantMismatch =
        kmodRaw is ModuleState.Installed &&
            !kmodRaw.active &&
            kernelRecommendation?.preferKmod == true &&
            recommendedKmi != null &&
            kmodRaw.gkiVariant != null &&
            kmodRaw.gkiVariant != recommendedKmi &&
            kmodRaw.gkiVariant != kernelRecommendation.alternativeGkiVariant
    val kmodUnknownVariantBroken =
        kmodRaw is ModuleState.Installed &&
            !kmodRaw.active &&
            kmodRaw.gkiVariant == null &&
            kernelRecommendation?.preferKmod == true
    val kmodOnUnsupportedKernel =
        kmodRaw is ModuleState.Installed &&
            !kmodRaw.active &&
            kernelRecommendation != null &&
            !kernelRecommendation.preferKmod
    // User installed one of the two candidates for an ambiguous GKI series
    // (5.10 / 5.15) and it failed to load this boot — suggest the other.
    val kmodAmbiguousLoadFailed =
        kmodRaw is ModuleState.Installed &&
            !kmodRaw.active &&
            kmodLoadStatus?.freshForCurrentBoot == true &&
            kernelRecommendation?.variantAmbiguous == true &&
            kmodRaw.gkiVariant != null &&
            (
                kmodRaw.gkiVariant == kernelRecommendation.recommendedGkiVariant ||
                    kmodRaw.gkiVariant == kernelRecommendation.alternativeGkiVariant
            )
    val kprobesMissing =
        kmodLoadStatus?.freshForCurrentBoot == true && kmodLoadStatus.kretprobes == "n"
    // Priority matches the Error emission below so the card color agrees
    // with the banner: kprobes-missing first, then unsupported-kernel,
    // then wrong-variant, then unknown-variant-inactive, then
    // ambiguous-load-failed (falls to this only when the installed kmod is
    // one of the ambiguous candidates — the mismatch branches above already
    // excluded both candidates from "wrong variant").
    val kmodBrokenReason: KmodBrokenReason? =
        when {
            kmodRaw !is ModuleState.Installed -> null
            kprobesMissing -> KmodBrokenReason.MissingKprobes
            kmodOnUnsupportedKernel -> KmodBrokenReason.UnsupportedKernel
            kmodVariantMismatch -> KmodBrokenReason.WrongVariant
            kmodUnknownVariantBroken -> KmodBrokenReason.UnknownVariantInactive
            kmodAmbiguousLoadFailed -> KmodBrokenReason.AmbiguousLoadFailed
            else -> null
        }
    val kmod: ModuleState =
        if (kmodRaw is ModuleState.Installed && kmodBrokenReason != null) {
            kmodRaw.copy(brokenReason = kmodBrokenReason)
        } else {
            kmodRaw
        }
    VpnHideLog.i(TAG, "kmod (with brokenReason): $kmod")
    // Only surface the blue "what to install" card when nothing is
    // installed yet. Wrong-variant / broken / unsupported-kernel cases
    // already emit a red error below with the same CTA — showing both
    // duplicates the instruction.
    val nativeInstallRecommendation =
        kernelRecommendation?.takeIf {
            kmod is ModuleState.NotInstalled && zygisk is ModuleState.NotInstalled
        }
    VpnHideLog.i(
        TAG,
        "nativeInstallRecommendation=$nativeInstallRecommendation " +
            "(raw=$kernelRecommendation variantMismatch=$kmodVariantMismatch " +
            "unknownVariantBroken=$kmodUnknownVariantBroken)",
    )

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
    val moduleMismatches =
        detectModuleMismatches(
            listOf(
                kmod to NativeModuleKind.Kmod,
                zygisk to NativeModuleKind.Zygisk,
                ports to NativeModuleKind.Ports,
            ),
            appVersion,
        )
    moduleMismatches.forEach { mismatch ->
        warn(buildModuleVersionIssue(mismatch.kind, mismatch.moduleVersion, mismatch.appVersion))
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
        if (versionsMismatch(runningVersion, appVersion)) {
            VpnHideLog.w(TAG, "version mismatch: running=$runningVersion app=$appVersion")
            warn(res.getString(R.string.dashboard_issue_version_mismatch, runningVersion, appVersion))
        }
    }

    // ── Warnings: suboptimal-but-working setups ──

    // W1: kernel supports kmod, but user only installed zygisk. Zygisk is
    // detected by banking / payment apps, so a user has to remember Z-off
    // per such app; kmod is invisible to anti-tamper.
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

    // W2: kmod and zygisk both active simultaneously — same coverage,
    // but Zygisk adds the per-app footgun for banking / payment targets.
    if (kmod is ModuleState.Installed &&
        kmod.active &&
        zygisk is ModuleState.Installed &&
        zygisk.active
    ) {
        warn(res.getString(R.string.dashboard_issue_both_native_active))
    }

    // W3: user has debug logging turned on — VPN Hide is writing verbose lines
    // to logcat that a forensic reader with root can see. The flag file is
    // written by the Diagnostics → Debug logging toggle; absent file ⇒
    // default off ⇒ no warning.
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

    // W5: VPN Hide installed in more than one user profile (work profile,
    // MIUI Second Space, etc.). Each instance can write to the shared
    // target files, but each one's app picker only sees apps from its own
    // profile (PackageManager.getInstalledApplications is per-user). A
    // Save from a profile that doesn't see all the targets would silently
    // drop them. Recommend uninstalling everywhere except the main profile.
    val (_, selfPmRaw) =
        suExec(
            "ALL_PKGS=\"\$(pm list packages -U --user all 2>/dev/null)\"; " +
                buildPackageUidsExpression(selfPkg, "SELF_UIDS") +
                "; echo \"\$SELF_UIDS\"",
        )
    val selfUidCount =
        selfPmRaw
            .lines()
            .count { it.trim().toIntOrNull() != null }
    if (selfUidCount > 1) {
        warn(res.getString(R.string.dashboard_issue_self_multi_profile, selfUidCount))
    }

    // ── Errors: kmod variant / load problems ──
    // Priority ordered: kprobes-missing first (no variant will ever work),
    // then "kernel has no kmod variant" (user picked the wrong tool),
    // then wrong-variant (concrete mismatch we can name), then unknown-variant
    // (old build that didn't stamp gkiVariant), then generic load failure
    // when we have insmod stderr to show. Only one kmod-failure issue fires
    // to avoid piling up related banners. All Errors — these mean kmod is
    // actively broken, not just suboptimal.
    val recommendedArtifact = kernelRecommendation?.recommendedArtifact
    if (kmod is ModuleState.Installed) {
        when {
            kmodLoadStatus?.freshForCurrentBoot == true &&
                kmodLoadStatus.kretprobes == "n" -> {
                err(res.getString(R.string.dashboard_issue_kprobes_missing))
            }

            kmodOnUnsupportedKernel && recommendedArtifact != null -> {
                err(
                    res.getString(
                        R.string.dashboard_issue_kmod_not_supported_kernel,
                        kmodLoadStatus?.unameR ?: "?",
                        recommendedArtifact,
                    ),
                )
            }

            kmodVariantMismatch -> {
                err(
                    res.getString(
                        R.string.dashboard_issue_kmod_wrong_variant,
                        (kmod as ModuleState.Installed).gkiVariant ?: "?",
                        recommendedKmi ?: "?",
                        recommendedArtifact ?: "?",
                    ),
                )
            }

            kmodUnknownVariantBroken && recommendedArtifact != null -> {
                err(
                    res.getString(
                        R.string.dashboard_issue_kmod_unknown_variant,
                        recommendedArtifact,
                    ),
                )
            }

            kmodAmbiguousLoadFailed -> {
                val installed = (kmod as ModuleState.Installed).gkiVariant
                val tryArtifact =
                    if (installed == kernelRecommendation?.recommendedGkiVariant) {
                        kernelRecommendation.alternativeArtifact
                    } else {
                        kernelRecommendation?.recommendedArtifact
                    }
                err(
                    res.getString(
                        R.string.dashboard_issue_kmod_ambiguous_try_alternative,
                        installed ?: "?",
                        tryArtifact ?: "?",
                    ),
                )
            }

            !kmod.active &&
                kmodLoadStatus?.freshForCurrentBoot == true &&
                kmodLoadStatus.insmodStderr != null -> {
                err(
                    res.getString(
                        R.string.dashboard_issue_kmod_load_failed,
                        kmodLoadStatus.insmodStderr,
                    ),
                )
            }
        }
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
        kmodLoadStatus = kmodLoadStatus,
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

    val info = caps.transportInfo
    val isVpnTi = info?.javaClass?.name?.contains("VpnTransportInfo") == true
    if (isVpnTi) failed++
    VpnHideLog.d(TAG, "java: transportInfo=${info?.javaClass?.name} isVpn=$isVpnTi")

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
