package dev.okhsunrog.vpnhide.picker

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.drawable.Drawable
import android.net.VpnService
import android.os.Build
import android.os.Process
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.StateCache
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.bit
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
 * Surfaced by [AppListCache.scanWarning] when the merged inventory is
 * [PackageInventory.partial] — some profile other than user 0 (the
 * in-process backstop covers user 0) didn't scan cleanly. Distinct from
 * [StateCache.error]: the app list itself loaded fine, this only flags that
 * part of it may be missing.
 */
internal data class PackageScanWarning(
    val failedUserIds: Set<Int>,
)

/**
 * Append a profile list to an app label so users can tell that
 * Telegram-in-Second-Space and Telegram-in-main are the same target.
 * Suppresses the suffix when the app is only in the current profile —
 * otherwise every row in the list reads "Cromite (0)", "Chrome (0)",
 * ... for users who don't even have a secondary profile.
 *
 * When [userNames] contains an entry for a user ID, its display name
 * ("Work", "Cloned app") is used; otherwise we fall back to the raw
 * numeric ID. This keeps the helper usable even before the user-name
 * map is loaded (no root / parse failure) — see
 * [AppListCache.userNames] for how nameless profiles get a name.
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
    logTag = LogTags.APP_LIST,
) {
    val apps: StateFlow<List<AppSummary>?> get() = value

    /** user_id → display profile name (e.g. 10 → "Work"). Populated
     * from `pm list users` alongside the package scan. Profiles the OS
     * reports without a name — clone profiles are routinely nameless —
     * get a name derived from their type ("Cloned app", "Private
     * space") instead of leaking a bare user ID into the UI. Empty map
     * if root isn't available or parsing failed — `labelWithUsers`
     * falls back to numeric IDs in that case.
     */
    private val _userNames = MutableStateFlow<Map<Int, String>>(emptyMap())
    val userNames: StateFlow<Map<Int, String>> = _userNames.asStateFlow()

    /** Non-null when the last load's inventory was partial for a profile
     * other than user 0. `StateCache.error` conflates "load failed" with
     * "load succeeded", so partiality needs its own flow — the picker keeps
     * showing the list and renders a soft banner instead of the hard-fail
     * card. */
    private val _scanWarning = MutableStateFlow<PackageScanWarning?>(null)
    val scanWarning: StateFlow<PackageScanWarning?> = _scanWarning.asStateFlow()

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
        _scanWarning.value = null
        RootSnapshotCache.invalidate()
        forceRefresh(scope)
    }

    override fun invalidate() {
        _scanWarning.value = null
        super.invalidate()
    }

    suspend fun loadForAgent(
        context: Context,
        force: Boolean,
    ): List<AppSummary> {
        appContext = context.applicationContext
        return if (!force) {
            apps.value ?: load(force = false)
        } else {
            RootSnapshotCache.invalidate()
            load(force = true)
        }
    }

    override suspend fun load(
        @Suppress("UNUSED_PARAMETER") force: Boolean,
    ): List<AppSummary> {
        val appContext = requireNotNull(appContext) { "AppListCache.load before ensureLoaded/refresh" }
        return withContext(Dispatchers.IO) {
            val pm = appContext.packageManager
            val vpnServicePkgs = queryVpnServiceProviders(pm)
            val sections = RootSnapshotCache.getOrLoad().sections
            val rawInventory =
                parsePackageInventory(
                    packagesRaw = sections["pm_packages"].orEmpty(),
                    usersRaw = sections["pm_users"].orEmpty(),
                )
            // 100_000: Android's per-user UID stride (also used by labelWithUsers below).
            val currentUserId = Process.myUid() / 100_000
            val mergedPackages =
                mergeUser0Backstop(
                    packages = rawInventory.packages,
                    user0Packages = queryUser0Backstop(pm),
                    currentUserId = currentUserId,
                )
            val inventory = rawInventory.copy(packages = mergedPackages).requireNonEmpty()
            _userNames.value =
                inventory.profiles.mapValues { (_, profile) -> profileDisplayName(appContext, profile) }
            updateScanWarning(inventory, currentUserId, _userNames.value)
            inventory.packages.entries
                .map { (pkg, meta) ->
                    val info = runCatching { pm.getApplicationInfo(pkg, 0) }.getOrNull()
                    val archiveInfo =
                        if (info == null) loadArchiveApplicationInfo(pm, meta.apkPath) else null
                    val effectiveInfo = info ?: archiveInfo

                    // Archive-parsed ApplicationInfo doesn't carry FLAG_SYSTEM
                    // (that bit is attached by PM at install time, not stored in
                    // the manifest), so use the APK path for secondary-only apps.
                    val isSystem =
                        if (info != null) {
                            (info.flags and ApplicationInfo.FLAG_SYSTEM) != 0
                        } else {
                            !meta.apkPath.orEmpty().startsWith("/data/app/")
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
        }
    }

    /**
     * Log-only + [scanWarning] surface for [PackageInventory.partial]. The
     * backstop already covers user 0, so only failures for other profiles
     * are worth flagging — those are the ones the app genuinely couldn't
     * enumerate this run.
     */
    private fun updateScanWarning(
        inventory: PackageInventory,
        user0Id: Int,
        profileNames: Map<Int, String>,
    ) {
        val otherProfileFailures = inventory.failedUserIds - user0Id
        if (otherProfileFailures.isNotEmpty()) {
            VpnHideLog.w(LogTags.APP_LIST, inventory.partialMessage(profileNames))
        }
        _scanWarning.value = otherProfileFailures.takeIf { it.isNotEmpty() }?.let(::PackageScanWarning)
    }

    /**
     * Cheap in-process backstop so the picker still shows *something* even
     * when the per-user root scan comes back completely empty for user 0 —
     * e.g. a total root-shell failure. Root can still see more (other
     * profiles, apps hidden from this process), which is why the per-user
     * scan stays primary; this only fills the user-0 gap.
     */
    private fun queryUser0Backstop(pm: PackageManager): List<BackstopPackage> {
        val infos =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                pm.getInstalledApplications(PackageManager.ApplicationInfoFlags.of(0L))
            } else {
                @Suppress("DEPRECATION")
                pm.getInstalledApplications(0)
            }
        return infos.map { info ->
            BackstopPackage(packageName = info.packageName, apkPath = info.sourceDir, uid = info.uid)
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

    /**
     * A profile's own name wins — it is what the system UI shows. Only
     * when the OS reports none do we synthesize one from the profile
     * type, so the Hiding list never shows a bare `(10)` the user can't
     * place against anything on their device.
     */
    private fun profileDisplayName(
        context: Context,
        profile: UserProfileInfo,
    ): String {
        profile.name?.let { return it }
        return when (profile.kind) {
            UserProfileKind.WORK -> context.getString(R.string.profile_kind_work)
            UserProfileKind.CLONE -> context.getString(R.string.profile_kind_clone)
            UserProfileKind.PRIVATE -> context.getString(R.string.profile_kind_private)
            UserProfileKind.SECONDARY -> context.getString(R.string.profile_kind_secondary, profile.id)
            UserProfileKind.UNKNOWN -> context.getString(R.string.profile_kind_unknown, profile.id)
        }
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
