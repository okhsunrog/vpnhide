package dev.okhsunrog.vpnhide.hook

import android.content.pm.ApplicationInfo
import android.content.pm.PackageInfo
import android.content.pm.ResolveInfo
import android.os.Binder
import android.os.Build
import android.os.FileObserver
import android.os.Process
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers
import dev.okhsunrog.vpnhide.LsposedStats
import dev.okhsunrog.vpnhide.generated.HookIds

/**
 * Package-visibility policy — hide packages from selected callers.
 *
 * Two flat groups:
 *   - hiddenPackages: package names to hide from PM responses
 *   - observerUids: caller UIDs that should not see hidden packages
 *
 * When the Binder caller in system_server is an observer, results of
 * PackageManager queries are filtered to exclude hidden packages:
 *   - list queries (getInstalledPackages / queryIntentActivities / ...)
 *     have matching entries removed from the returned ParceledListSlice
 *   - single-package queries (getPackageInfo / getApplicationInfo /
 *     resolveService / ...) return null, which the caller side converts
 *     to NameNotFoundException
 *
 * Targets the PackageManagerService Binder stub via
 * com.android.server.pm.IPackageManagerBase — same file in AOSP 13/14/15.
 * Filtering happens post-AppsFilter (AppsFilter runs inside ComputerEngine,
 * before these methods return), so we subtract further.
 *
 * System callers (UID < 10000) are always exempt to avoid breaking
 * installd, LauncherApps, StatusBar, etc.
 */
internal object PackageVisibilityHooks {
    private const val IPM_BASE = "com.android.server.pm.IPackageManagerBase"
    private const val IPM_LEGACY = "com.android.server.pm.PackageManagerService"
    private const val PARCELED_LIST_SLICE = "android.content.pm.ParceledListSlice"

    private data class ObserverCaller(
        val uid: Int,
        val appId: Int,
        val config: SystemServerConfig,
    )

    @Volatile private var parceledListSliceClass: Class<*>? = null

    @Volatile private var fileObserver: FileObserver? = null

    fun install(classLoader: ClassLoader) {
        val ipmClass =
            try {
                classLoader.loadClass(IPM_BASE)
            } catch (_: ClassNotFoundException) {
                try {
                    classLoader.loadClass(IPM_LEGACY)
                } catch (t: Throwable) {
                    HookLog.e("VpnHide/PV: neither $IPM_BASE nor $IPM_LEGACY found: ${t.message}")
                    return
                }
            }
        HookLog.i("VpnHide/PV: hooking ${ipmClass.name}")

        parceledListSliceClass =
            try {
                classLoader.loadClass(PARCELED_LIST_SLICE)
            } catch (t: Throwable) {
                HookLog.e("VpnHide/PV: ParceledListSlice not found: ${t.message}")
                return
            }

        hook(
            ipmClass,
            "getInstalledPackages",
            listFilter<PackageInfo>("getInstalledPackages") { it.packageName },
        )
        hook(
            ipmClass,
            "getInstalledApplications",
            listFilter<ApplicationInfo>("getInstalledApplications") { it.packageName },
        )
        hook(ipmClass, "queryIntentActivities", resolveInfoListFilter("queryIntentActivities"))
        hook(ipmClass, "queryIntentServices", resolveInfoListFilter("queryIntentServices"))
        hook(ipmClass, "queryIntentReceivers", resolveInfoListFilter("queryIntentReceivers"))
        hook(ipmClass, "queryIntentContentProviders", resolveInfoListFilter("queryIntentContentProviders"))

        hook(ipmClass, "getPackageInfo", singleHideByFirstStringArg("getPackageInfo"))
        hook(ipmClass, "getApplicationInfo", singleHideByFirstStringArg("getApplicationInfo"))
        hook(ipmClass, "getInstallerPackageName", singleHideByFirstStringArg("getInstallerPackageName"))
        hook(
            ipmClass,
            "getInstallSourceInfo",
            singleHideByFirstStringArg("getInstallSourceInfo"),
            minApi = Build.VERSION_CODES.R,
        )
        hook(ipmClass, "getPackageUid", packageUidHide())
        hook(ipmClass, "resolveIntent", resolveInfoSingleHide("resolveIntent"))
        hook(ipmClass, "resolveService", resolveInfoSingleHide("resolveService"))
        hook(ipmClass, "getPackagesForUid", packagesForUidHide())
        hook(ipmClass, "getNameForUid", nameForUidHide())
        hook(ipmClass, "getNamesForUids", namesForUidsHide())

        watchConfigFiles()
    }

    // ------------------------------------------------------------------
    //  Caller classification
    // ------------------------------------------------------------------

    private fun observerCaller(): ObserverCaller? {
        val uid = Binder.getCallingUid()
        // Exempt system callers: installd, shell, system_server itself,
        // LauncherApps, StatusBar, etc. all run under UID < 10000.
        if (uid < Process.FIRST_APPLICATION_UID) return null
        if (uid == Process.myUid()) return null
        val appId = SystemServerConfigCache.appId(uid)
        val config = SystemServerConfigCache.load()
        return if (config.observerAppIds.contains(appId)) ObserverCaller(uid, appId, config) else null
    }

    private fun ObserverCaller.shouldHidePackage(packageName: String): Boolean = config.shouldHidePackageForCallerAppId(packageName, appId)

    private fun watchConfigFiles() {
        fileObserver =
            watchSystemDataDir { path ->
                if (path == "vpnhide_config.json") {
                    HookLog.i("VpnHide/PV: canonical config changed, invalidating")
                    SystemServerConfigCache.invalidate()
                }
            }
        HookLog.i("VpnHide/PV: watching /data/system for canonical config changes")
    }

    // ------------------------------------------------------------------
    //  Hook installation
    // ------------------------------------------------------------------

    private fun hook(
        clazz: Class<*>,
        methodName: String,
        handler: XC_MethodHook,
        // AOSP API level that introduced the method. Below it the method (and the
        // detection vector it covers) doesn't exist, so a missing target is
        // expected — logged at INFO instead of ERROR. 0 = present on all supported.
        minApi: Int = 0,
    ) {
        try {
            val hooked = XposedBridge.hookAllMethods(clazz, methodName, handler)
            if (hooked.isEmpty()) {
                if (minApi > 0 && Build.VERSION.SDK_INT < minApi) {
                    HookLog.i("VpnHide/PV: $methodName absent on API ${Build.VERSION.SDK_INT} (added in API $minApi)")
                } else {
                    HookLog.e("VpnHide/PV: no method '$methodName' on ${clazz.name}")
                }
            } else {
                HookLog.i("VpnHide/PV: hooked $methodName (${hooked.size} overload(s))")
            }
        } catch (t: Throwable) {
            HookLog.e("VpnHide/PV: hook $methodName failed: ${t::class.java.simpleName}: ${t.message}")
        }
    }

    // ------------------------------------------------------------------
    //  Hook handlers
    // ------------------------------------------------------------------

    /**
     * Generic list filter for ParceledListSlice<T>.
     * Removes items whose packageName (extracted by [pkgOf]) is in hiddenPackages.
     */
    private inline fun <reified T> listFilter(
        methodName: String,
        crossinline pkgOf: (T) -> String?,
    ): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                val caller = observerCaller() ?: return
                if (caller.config.hiddenPackages.isEmpty()) return

                val result = param.result ?: return
                val pls = parceledListSliceClass ?: return
                if (!pls.isInstance(result)) return

                @Suppress("UNCHECKED_CAST")
                val original = XposedHelpers.callMethod(result, "getList") as? List<T> ?: return
                val filtered =
                    original.filter {
                        val p = pkgOf(it)
                        p == null || !caller.shouldHidePackage(p)
                    }
                val removed = original.mapNotNull { pkgOf(it)?.takeIf { pkg -> caller.shouldHidePackage(pkg) } }
                if (removed.isEmpty()) return

                param.result = newListResultLike(result, filtered) ?: return
                LsposedStats.record(caller.uid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                HookLog.i(
                    "VpnHide/PV: $methodName uid=${caller.uid} filtered ${removed.size}/${original.size} " +
                        "hidden=${removed.sorted()} wrapper=${result.javaClass.simpleName}",
                )
            }
        }

    /**
     * ResolveInfo list: packageName is on the inner ComponentInfo
     * (activityInfo / serviceInfo / providerInfo).
     */
    private fun resolveInfoListFilter(methodName: String): XC_MethodHook =
        listFilter<ResolveInfo>(methodName) { resolveInfoPackageName(it) }

    /**
     * Keep the exact runtime wrapper class returned by PackageManager. Android
     * 17 wraps getInstalledPackages in PackageInfoList, a ParceledListSlice
     * subclass with a custom ParcelDedupHelper. Replacing it with the base
     * ParceledListSlice corrupts the client-side parcel stream.
     */
    private fun newListResultLike(
        originalResult: Any,
        filtered: List<*>,
    ): Any? =
        try {
            XposedHelpers.newInstance(originalResult.javaClass, filtered)
        } catch (t: Throwable) {
            HookLog.e(
                "VpnHide/PV: failed to create filtered ${originalResult.javaClass.name}: ${t.message}",
            )
            null
        }

    private fun resolveInfoPackageName(ri: ResolveInfo): String? =
        ri.activityInfo?.packageName
            ?: ri.serviceInfo?.packageName
            ?: ri.providerInfo?.packageName

    /**
     * For getPackageInfo / getApplicationInfo / getInstallerPackageName / getInstallSourceInfo.
     * Signature starts with `String packageName`. If that package is hidden and caller is an
     * observer, set result=null. Caller-side API converts null to NameNotFoundException.
     */
    private fun singleHideByFirstStringArg(methodName: String): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                if (param.result == null) return
                val pkg = param.args.firstOrNull() as? String ?: return
                val caller = observerCaller() ?: return
                if (caller.shouldHidePackage(pkg)) {
                    param.result = null
                    LsposedStats.record(caller.uid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                    HookLog.i("VpnHide/PV: $methodName uid=${caller.uid} hid $pkg")
                }
            }
        }

    /** getPackageUid(String, long/int, int): returns -1 if hidden. */
    private fun packageUidHide(): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                val pkg = param.args.firstOrNull() as? String ?: return
                val caller = observerCaller() ?: return
                if (caller.shouldHidePackage(pkg)) {
                    param.result = -1
                    LsposedStats.record(caller.uid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                    HookLog.i("VpnHide/PV: getPackageUid uid=${caller.uid} hid $pkg")
                }
            }
        }

    /** resolveIntent / resolveService: ResolveInfo result. Null it out if it points to a hidden pkg. */
    private fun resolveInfoSingleHide(methodName: String): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                val ri = param.result as? ResolveInfo ?: return
                val caller = observerCaller() ?: return
                val pkg = resolveInfoPackageName(ri) ?: return
                if (caller.shouldHidePackage(pkg)) {
                    param.result = null
                    LsposedStats.record(caller.uid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                    HookLog.i("VpnHide/PV: $methodName uid=${caller.uid} hid $pkg")
                }
            }
        }

    /** getPackagesForUid(int): String[]. Filter out hidden entries. Return null if all filtered. */
    private fun packagesForUidHide(): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                val arr = param.result as? Array<*> ?: return
                val caller = observerCaller() ?: return
                if (caller.config.hiddenPackages.isEmpty()) return
                val filtered = arr.filterIsInstance<String>().filterNot { caller.shouldHidePackage(it) }
                if (filtered.size == arr.size) return
                param.result = if (filtered.isEmpty()) null else filtered.toTypedArray()
                LsposedStats.record(caller.uid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                val requestedUid = param.args.firstOrNull()
                val removed = arr.filterIsInstance<String>().filter { caller.shouldHidePackage(it) }
                HookLog.i(
                    "VpnHide/PV: getPackagesForUid uid=${caller.uid} requestedUid=$requestedUid " +
                        "hidden=${removed.sorted()}",
                )
            }
        }

    /**
     * getNameForUid(int): the single package name owning a uid. Null it out
     * when hidden, so a hidden package can't be discovered by resolving a uid
     * back to its name (the dual of getPackagesForUid).
     */
    private fun nameForUidHide(): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                val name = param.result as? String ?: return
                val caller = observerCaller() ?: return
                if (caller.shouldHidePackage(name)) {
                    param.result = null
                    LsposedStats.record(caller.uid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                    HookLog.i("VpnHide/PV: getNameForUid uid=${caller.uid} hid $name")
                }
            }
        }

    /**
     * getNamesForUids(int[]): names positional per requested uid. Null the
     * hidden entries in place (the array is the result we return), so a bulk
     * uid→name resolve can't enumerate a hidden package either.
     */
    private fun namesForUidsHide(): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                @Suppress("UNCHECKED_CAST")
                val names = param.result as? Array<String?> ?: return
                val caller = observerCaller() ?: return
                if (caller.config.hiddenPackages.isEmpty()) return
                var changed = false
                for (i in names.indices) {
                    val name = names[i] ?: continue
                    if (caller.shouldHidePackage(name)) {
                        names[i] = null
                        changed = true
                    }
                }
                if (changed) {
                    LsposedStats.record(caller.uid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                    HookLog.i("VpnHide/PV: getNamesForUids uid=${caller.uid} hid some entries")
                }
            }
        }
}
