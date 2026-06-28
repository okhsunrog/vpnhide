package dev.okhsunrog.vpnhide

import android.content.pm.ApplicationInfo
import android.content.pm.PackageInfo
import android.content.pm.ResolveInfo
import android.os.Binder
import android.os.FileObserver
import android.os.Process
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers
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
        hook(ipmClass, "getInstallSourceInfo", singleHideByFirstStringArg("getInstallSourceInfo"))
        hook(ipmClass, "getPackageUid", packageUidHide())
        hook(ipmClass, "resolveIntent", resolveInfoSingleHide("resolveIntent"))
        hook(ipmClass, "resolveService", resolveInfoSingleHide("resolveService"))
        hook(ipmClass, "getPackagesForUid", packagesForUidHide())

        watchConfigFiles()
    }

    // ------------------------------------------------------------------
    //  Caller classification
    // ------------------------------------------------------------------

    private fun observerCallerUid(): Int? {
        val uid = Binder.getCallingUid()
        // Exempt system callers: installd, shell, system_server itself,
        // LauncherApps, StatusBar, etc. all run under UID < 10000.
        if (uid < Process.FIRST_APPLICATION_UID) return null
        if (uid == Process.myUid()) return null
        val appId = SystemServerConfigCache.appId(uid)
        return if (SystemServerConfigCache.load().observerAppIds.contains(appId)) uid else null
    }

    private fun loadHiddenPackages(): Set<String> = SystemServerConfigCache.load().hiddenPackages

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
    ) {
        try {
            val hooked = XposedBridge.hookAllMethods(clazz, methodName, handler)
            if (hooked.isEmpty()) {
                HookLog.e("VpnHide/PV: no method '$methodName' on ${clazz.name}")
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
                val callerUid = observerCallerUid() ?: return
                val hidden = loadHiddenPackages()
                if (hidden.isEmpty()) return

                val result = param.result ?: return
                val pls = parceledListSliceClass ?: return
                if (!pls.isInstance(result)) return

                @Suppress("UNCHECKED_CAST")
                val original = XposedHelpers.callMethod(result, "getList") as? List<T> ?: return
                val filtered =
                    original.filter {
                        val p = pkgOf(it)
                        p == null || p !in hidden
                    }
                val removed = original.mapNotNull { pkgOf(it)?.takeIf(hidden::contains) }
                if (removed.isEmpty()) return

                param.result = newListResultLike(result, filtered) ?: return
                LsposedStats.record(callerUid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                HookLog.i(
                    "VpnHide/PV: $methodName uid=$callerUid filtered ${removed.size}/${original.size} " +
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
                val callerUid = observerCallerUid() ?: return
                if (pkg in loadHiddenPackages()) {
                    param.result = null
                    LsposedStats.record(callerUid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                    HookLog.i("VpnHide/PV: $methodName uid=$callerUid hid $pkg")
                }
            }
        }

    /** getPackageUid(String, long/int, int): returns -1 if hidden. */
    private fun packageUidHide(): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                val pkg = param.args.firstOrNull() as? String ?: return
                val callerUid = observerCallerUid() ?: return
                if (pkg in loadHiddenPackages()) {
                    param.result = -1
                    LsposedStats.record(callerUid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                    HookLog.i("VpnHide/PV: getPackageUid uid=$callerUid hid $pkg")
                }
            }
        }

    /** resolveIntent / resolveService: ResolveInfo result. Null it out if it points to a hidden pkg. */
    private fun resolveInfoSingleHide(methodName: String): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                val ri = param.result as? ResolveInfo ?: return
                val callerUid = observerCallerUid() ?: return
                val pkg = resolveInfoPackageName(ri) ?: return
                if (pkg in loadHiddenPackages()) {
                    param.result = null
                    LsposedStats.record(callerUid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                    HookLog.i("VpnHide/PV: $methodName uid=$callerUid hid $pkg")
                }
            }
        }

    /** getPackagesForUid(int): String[]. Filter out hidden entries. Return null if all filtered. */
    private fun packagesForUidHide(): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                val arr = param.result as? Array<*> ?: return
                val callerUid = observerCallerUid() ?: return
                val hidden = loadHiddenPackages()
                if (hidden.isEmpty()) return
                val filtered = arr.filterIsInstance<String>().filter { it !in hidden }
                if (filtered.size == arr.size) return
                param.result = if (filtered.isEmpty()) null else filtered.toTypedArray()
                LsposedStats.record(callerUid, HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
                val requestedUid = param.args.firstOrNull()
                val removed = arr.filterIsInstance<String>().filter { it in hidden }
                HookLog.i(
                    "VpnHide/PV: getPackagesForUid uid=$callerUid requestedUid=$requestedUid " +
                        "hidden=${removed.sorted()}",
                )
            }
        }
}
