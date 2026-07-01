package dev.okhsunrog.vpnhide

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.drawable.Drawable
import android.net.VpnService
import android.os.Build
import android.os.Process
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import java.util.Locale

/**
 * Common per-installed-app fields used by the unified Apps list.
 * Role-specific state (Java, Native, Apps, Ports) is merged with this data
 * at render time.
 */
internal data class AppSummary(
    val packageName: String,
    val label: String,
    val icon: Drawable?,
    val isSystem: Boolean,
    val userIds: List<Int> = emptyList(),
    val declaresVpnService: Boolean = false,
    val nameContainsVpn: Boolean = false,
)

internal fun AppSummary.toAutoHideSignal(): AppAutoHideSignal =
    AppAutoHideSignal(
        packageName = packageName,
        declaresVpnService = declaresVpnService,
        nameContainsVpn = nameContainsVpn,
    )

internal fun looksLikeVpnAppName(label: String): Boolean = label.uppercase(Locale.ROOT).contains("VPN")

/**
 * Append a profile list to an app label so users can tell that
 * Telegram-in-Second-Space and Telegram-in-main are the same target.
 * Suppresses the suffix when the app is only in the current profile —
 * otherwise every row in the list reads "Cromite (0)", "Chrome (0)",
 * ... for users who don't even have a secondary profile.
 *
 * When [userNames] contains an entry for a user ID, its friendly name
 * ("Work", "Second Space") is used; otherwise we fall back to the raw
 * numeric ID. This keeps the helper usable even before the user-name
 * map is loaded (no root / parse failure).
 */
internal fun labelWithUsers(
    label: String,
    userIds: List<Int>,
    userNames: Map<Int, String> = emptyMap(),
): String {
    if (userIds.isEmpty()) return label
    val currentUser = Process.myUid() / 100000
    val onlyCurrent = userIds.size == 1 && userIds[0] == currentUser
    if (onlyCurrent) return label
    val formatted = userIds.joinToString(", ") { userNames[it] ?: it.toString() }
    return "$label ($formatted)"
}

/**
 * App-scoped cache for the installed-app list. Loaded asynchronously
 * at startup; the Apps screen subscribes to `apps` and renders
 * instantly on tab switch. The top-bar refresh button calls [refresh]
 * which reloads the package + icon list; the Apps screen re-merges
 * their per-screen target flags reactively off `apps` + the targets
 * snapshot, so nothing keys off a manual refresh counter anymore.
 */
internal object AppListCache : StateCache<List<AppSummary>>(
    traceName = "app_list_cache",
    logTag = "VpnHide-AppList",
) {
    val apps: StateFlow<List<AppSummary>?> get() = value

    /** user_id → friendly profile name (e.g. 10 → "Work"). Populated
     * from `pm list users` alongside the package scan. Empty map if
     * root isn't available or parsing failed — `labelWithUsers` falls
     * back to numeric IDs in that case.
     */
    private val _userNames = MutableStateFlow<Map<Int, String>>(emptyMap())
    val userNames: StateFlow<Map<Int, String>> = _userNames.asStateFlow()

    @Volatile private var appContext: Context? = null

    /** Kick off an initial load if not already loaded or loading. */
    fun ensureLoaded(
        scope: CoroutineScope,
        context: Context,
    ) {
        appContext = context.applicationContext
        ensure(scope)
    }

    /** Force a reload of the package + icon list. */
    fun refresh(
        scope: CoroutineScope,
        context: Context,
    ) {
        appContext = context.applicationContext
        forceRefresh(scope)
    }

    suspend fun loadForAgent(
        context: Context,
        force: Boolean,
    ): List<AppSummary> {
        appContext = context.applicationContext
        return if (!force) {
            apps.value ?: load(force = false)
        } else {
            load(force = true)
        }
    }

    override suspend fun load(force: Boolean): List<AppSummary> {
        val appContext = requireNotNull(appContext) { "AppListCache.load before ensureLoaded/refresh" }
        return withContext(Dispatchers.IO) {
            val pm = appContext.packageManager
            val vpnServicePkgs = queryVpnServiceProviders(pm)
            val (packages, users) = loadPackagesAndUsersViaRoot()
            _userNames.value = users
            if (packages.isNotEmpty()) {
                packages.entries
                    .map { (pkg, meta) ->
                        val info = runCatching { pm.getApplicationInfo(pkg, 0) }.getOrNull()
                        val archiveInfo =
                            if (info == null) loadArchiveApplicationInfo(pm, meta.apkPath) else null
                        val effectiveInfo = info ?: archiveInfo

                        // Archive-parsed ApplicationInfo doesn't carry
                        // FLAG_SYSTEM (that bit is attached by PM at
                        // install time, not stored in the manifest), so
                        // for secondary-only packages we'd misclassify
                        // every system app as user-installed. Fall back
                        // to the APK path: /data/app/... is user-
                        // installed, everything else is baked into the
                        // system image.
                        val isSystem =
                            if (info != null) {
                                (info.flags and ApplicationInfo.FLAG_SYSTEM) != 0
                            } else {
                                !meta.apkPath.startsWith("/data/app/")
                            }
                        val label = effectiveInfo?.loadLabel(pm)?.toString() ?: pkg

                        AppSummary(
                            packageName = pkg,
                            label = label,
                            icon = effectiveInfo?.let { runCatching { pm.getApplicationIcon(it) }.getOrNull() },
                            isSystem = isSystem,
                            userIds = meta.userIds,
                            declaresVpnService = pkg in vpnServicePkgs,
                            nameContainsVpn = !isSystem && looksLikeVpnAppName(label),
                        )
                    }.sortedBy { it.label.lowercase() }
            } else {
                // Fallback: current-profile only (legacy behavior)
                pm
                    .getInstalledApplications(0)
                    .map { info ->
                        val label = info.loadLabel(pm).toString()
                        val isSystem = (info.flags and ApplicationInfo.FLAG_SYSTEM) != 0
                        AppSummary(
                            packageName = info.packageName,
                            label = label,
                            icon = runCatching { pm.getApplicationIcon(info) }.getOrNull(),
                            isSystem = isSystem,
                            userIds = listOf(Process.myUid() / 100000),
                            declaresVpnService = info.packageName in vpnServicePkgs,
                            nameContainsVpn = !isSystem && looksLikeVpnAppName(label),
                        )
                    }.sortedBy { it.label.lowercase() }
            }
        }
    }

    private fun queryVpnServiceProviders(pm: PackageManager): Set<String> {
        val intent = Intent(VpnService.SERVICE_INTERFACE)
        val resolveInfos =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                pm.queryIntentServices(intent, PackageManager.ResolveInfoFlags.of(0L))
            } else {
                @Suppress("DEPRECATION")
                pm.queryIntentServices(intent, 0)
            }
        return resolveInfos
            .asSequence()
            .mapNotNull { resolveInfo ->
                val serviceInfo = resolveInfo.serviceInfo ?: return@mapNotNull null
                if (serviceInfo.permission != Manifest.permission.BIND_VPN_SERVICE) return@mapNotNull null
                serviceInfo.packageName?.takeIf { it.isNotBlank() }
            }.toSet()
    }

    private data class PkgMeta(
        val apkPath: String,
        val userIds: List<Int>,
    )

    private const val USERS_SENTINEL = "===VPNHIDE-USERS-BOUNDARY==="

    /**
     * Enumerate every installed package and every user profile in a
     * single `su` invocation. `pm list packages -U -f --user all` gives
     * APK path + UID per (pkg, user) tuple; `pm list users` gives the
     * friendly profile names ("Work", "Second Space") that the UI
     * renders instead of raw user IDs. Two commands, one `su` spawn —
     * separated by a sentinel the parser splits on.
     *
     * Packages output is one of:
     *   package:<apk_path>=<pkg> uid:<uid>            (single-user)
     *   package:<apk_path>=<pkg> uid:<uid>,<uid>,...  (AOSP --user all)
     *   package:<apk_path>=<pkg> uid:<uid>            (repeated per user
     *                                                  on some ROMs)
     * Users output is:
     *   UserInfo{<id>:<name>:<flags>} [running ...]
     */
    private fun loadPackagesAndUsersViaRoot(): Pair<Map<String, PkgMeta>, Map<Int, String>> {
        val (exitCode, raw) =
            suExec(
                "pm list packages -U -f --user all 2>/dev/null; " +
                    "echo '$USERS_SENTINEL'; " +
                    "pm list users 2>/dev/null",
            )
        if (exitCode != 0) return emptyMap<String, PkgMeta>() to emptyMap()
        val parts = raw.split(USERS_SENTINEL, limit = 2)
        val packages = parsePackages(parts[0])
        val users = if (parts.size > 1) parseUsers(parts[1]) else emptyMap()
        return packages to users
    }

    private fun parsePackages(raw: String): Map<String, PkgMeta> {
        val out = LinkedHashMap<String, PkgMeta>()
        raw
            .lineSequence()
            .map { it.trim() }
            .filter { it.startsWith("package:") }
            .forEach { line ->
                val body = line.removePrefix("package:")
                val uidMarker = body.lastIndexOf(" uid:")
                if (uidMarker <= 0) return@forEach
                val pathAndPkg = body.substring(0, uidMarker)
                val uidPart = body.substring(uidMarker + " uid:".length).trim()
                val eq = pathAndPkg.lastIndexOf('=')
                if (eq <= 0 || eq >= pathAndPkg.lastIndex) return@forEach
                val apkPath = pathAndPkg.substring(0, eq).trim()
                val pkg = pathAndPkg.substring(eq + 1).trim()
                if (apkPath.isEmpty() || pkg.isEmpty()) return@forEach

                val userIds =
                    uidPart
                        .split(',')
                        .mapNotNull { it.trim().toIntOrNull() }
                        .map { it / 100000 }

                val existing = out[pkg]
                out[pkg] =
                    if (existing == null) {
                        PkgMeta(apkPath, userIds.distinct().sorted())
                    } else {
                        existing.copy(
                            userIds = (existing.userIds + userIds).distinct().sorted(),
                        )
                    }
            }
        return out
    }

    // `UserInfo{10:Work:1030}` — flags are trailing hex, name is
    // everything between the first `:` and the last `:`. The lazy
    // name-group (.*?) combined with a greedy flags-group anchored to
    // `}` handles names that contain `:` (rare, but Android allows it).
    private val userLine = Regex("""UserInfo\{(\d+):(.*?):[0-9a-fA-F]+\}""")

    private fun parseUsers(raw: String): Map<Int, String> {
        val out = LinkedHashMap<Int, String>()
        raw.lineSequence().forEach { line ->
            val m = userLine.find(line) ?: return@forEach
            val id = m.groupValues[1].toIntOrNull() ?: return@forEach
            val name = m.groupValues[2].trim()
            if (name.isNotEmpty()) out[id] = name
        }
        return out
    }

    @Suppress("DEPRECATION")
    private fun loadArchiveApplicationInfo(
        pm: PackageManager,
        apkPath: String?,
    ): ApplicationInfo? {
        if (apkPath.isNullOrBlank()) return null
        val pkgInfo = runCatching { pm.getPackageArchiveInfo(apkPath, 0) }.getOrNull() ?: return null
        val appinfo = pkgInfo.applicationInfo ?: return null
        appinfo.sourceDir = apkPath
        appinfo.publicSourceDir = apkPath
        return appinfo
    }
}
