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
import java.io.File

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
    private const val HIDDEN_PKGS_FILE = "/data/system/vpnhide_hidden_pkgs.txt"
    private const val OBSERVER_UIDS_FILE = "/data/system/vpnhide_observer_uids.txt"
    private const val IPM_BASE = "com.android.server.pm.IPackageManagerBase"
    private const val IPM_LEGACY = "com.android.server.pm.PackageManagerService"
    private const val PARCELED_LIST_SLICE = "android.content.pm.ParceledListSlice"

    @Volatile private var parceledListSliceClass: Class<*>? = null

    @Volatile private var hiddenPackages: Set<String>? = null

    @Volatile private var observerUids: Set<Int>? = null

    @Volatile private var fileObserver: FileObserver? = null
    private val lock = Any()

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

        hook(ipmClass, "getInstalledPackages", listFilter<PackageInfo> { it.packageName })
        hook(ipmClass, "getInstalledApplications", listFilter<ApplicationInfo> { it.packageName })
        hook(ipmClass, "queryIntentActivities", resolveInfoListFilter())
        hook(ipmClass, "queryIntentServices", resolveInfoListFilter())
        hook(ipmClass, "queryIntentReceivers", resolveInfoListFilter())
        hook(ipmClass, "queryIntentContentProviders", resolveInfoListFilter())

        hook(ipmClass, "getPackageInfo", singleHideByFirstStringArg())
        hook(ipmClass, "getApplicationInfo", singleHideByFirstStringArg())
        hook(ipmClass, "getInstallerPackageName", singleHideByFirstStringArg())
        hook(ipmClass, "getInstallSourceInfo", singleHideByFirstStringArg())
        hook(ipmClass, "getPackageUid", packageUidHide())
        hook(ipmClass, "resolveIntent", resolveInfoSingleHide())
        hook(ipmClass, "resolveService", resolveInfoSingleHide())
        hook(ipmClass, "getPackagesForUid", packagesForUidHide())

        watchConfigFiles()
    }

    // ------------------------------------------------------------------
    //  Caller classification
    // ------------------------------------------------------------------

    private fun isObserverCaller(): Boolean {
        val uid = Binder.getCallingUid()
        // Exempt system callers: installd, shell, system_server itself,
        // LauncherApps, StatusBar, etc. all run under UID < 10000.
        if (uid < Process.FIRST_APPLICATION_UID) return false
        if (uid == Process.myUid()) return false
        return loadObserverUids().contains(uid)
    }

    private fun loadObserverUids(): Set<Int> {
        observerUids?.let { return it }
        synchronized(lock) {
            observerUids?.let { return it }
            val result = readUidFile(OBSERVER_UIDS_FILE)
            observerUids = result
            if (result.isNotEmpty()) {
                HookLog.i("VpnHide/PV: loaded ${result.size} observer UIDs: $result")
            }
            return result
        }
    }

    private fun loadHiddenPackages(): Set<String> {
        hiddenPackages?.let { return it }
        synchronized(lock) {
            hiddenPackages?.let { return it }
            val result = readLineFile(HIDDEN_PKGS_FILE)
            hiddenPackages = result
            if (result.isNotEmpty()) {
                HookLog.i("VpnHide/PV: loaded ${result.size} hidden packages: $result")
            }
            return result
        }
    }

    private fun readUidFile(path: String): Set<Int> =
        try {
            val f = File(path)
            if (!f.exists()) {
                emptySet()
            } else {
                f
                    .readLines()
                    .mapNotNull { it.trim().takeIf { s -> s.isNotEmpty() && !s.startsWith("#") }?.toIntOrNull() }
                    .toSet()
            }
        } catch (t: Throwable) {
            HookLog.e("VpnHide/PV: failed to read $path: ${t.message}")
            emptySet()
        }

    private fun readLineFile(path: String): Set<String> =
        try {
            val f = File(path)
            if (!f.exists()) {
                emptySet()
            } else {
                f
                    .readLines()
                    .map { it.trim() }
                    .filter { it.isNotEmpty() && !it.startsWith("#") }
                    .toSet()
            }
        } catch (t: Throwable) {
            HookLog.e("VpnHide/PV: failed to read $path: ${t.message}")
            emptySet()
        }

    private fun watchConfigFiles() {
        // FileObserver(File, Int) is API 29+; use the String-path form for API 28 compatibility.
        @Suppress("DEPRECATION")
        val observer =
            object : FileObserver("/data/system", CREATE or CLOSE_WRITE or MOVED_TO or MODIFY) {
                override fun onEvent(
                    event: Int,
                    path: String?,
                ) {
                    when (path) {
                        "vpnhide_hidden_pkgs.txt" -> {
                            HookLog.i("VpnHide/PV: hidden_pkgs changed, invalidating")
                            hiddenPackages = null
                        }

                        "vpnhide_observer_uids.txt" -> {
                            HookLog.i("VpnHide/PV: observer_uids changed, invalidating")
                            observerUids = null
                        }
                    }
                }
            }
        fileObserver = observer
        observer.startWatching()
        HookLog.i("VpnHide/PV: watching /data/system for config changes")
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
    private inline fun <reified T> listFilter(crossinline pkgOf: (T) -> String?): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                if (!isObserverCaller()) return
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
                if (filtered.size != original.size) {
                    param.result = XposedHelpers.newInstance(pls, filtered)
                }
            }
        }

    /** ResolveInfo list: packageName is on the inner ComponentInfo (activityInfo / serviceInfo / providerInfo). */
    private fun resolveInfoListFilter(): XC_MethodHook = listFilter<ResolveInfo> { resolveInfoPackageName(it) }

    private fun resolveInfoPackageName(ri: ResolveInfo): String? =
        ri.activityInfo?.packageName
            ?: ri.serviceInfo?.packageName
            ?: ri.providerInfo?.packageName

    /**
     * For getPackageInfo / getApplicationInfo / getInstallerPackageName / getInstallSourceInfo.
     * Signature starts with `String packageName`. If that package is hidden and caller is an
     * observer, set result=null. Caller-side API converts null to NameNotFoundException.
     */
    private fun singleHideByFirstStringArg(): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                if (param.result == null) return
                val pkg = param.args.firstOrNull() as? String ?: return
                if (!isObserverCaller()) return
                if (pkg in loadHiddenPackages()) {
                    param.result = null
                }
            }
        }

    /** getPackageUid(String, long/int, int): returns -1 if hidden. */
    private fun packageUidHide(): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                val pkg = param.args.firstOrNull() as? String ?: return
                if (!isObserverCaller()) return
                if (pkg in loadHiddenPackages()) {
                    param.result = -1
                }
            }
        }

    /** resolveIntent / resolveService: ResolveInfo result. Null it out if it points to a hidden pkg. */
    private fun resolveInfoSingleHide(): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                val ri = param.result as? ResolveInfo ?: return
                if (!isObserverCaller()) return
                val pkg = resolveInfoPackageName(ri) ?: return
                if (pkg in loadHiddenPackages()) {
                    param.result = null
                }
            }
        }

    /** getPackagesForUid(int): String[]. Filter out hidden entries. Return null if all filtered. */
    private fun packagesForUidHide(): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable()) return
                val arr = param.result as? Array<*> ?: return
                if (!isObserverCaller()) return
                val hidden = loadHiddenPackages()
                if (hidden.isEmpty()) return
                val filtered = arr.filterIsInstance<String>().filter { it !in hidden }
                if (filtered.size == arr.size) return
                param.result = if (filtered.isEmpty()) null else filtered.toTypedArray()
            }
        }
}
