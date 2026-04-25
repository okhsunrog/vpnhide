package dev.okhsunrog.vpnhide

import android.net.LinkProperties
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import android.net.RouteInfo
import android.os.Binder
import de.robv.android.xposed.IXposedHookLoadPackage
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedHelpers
import de.robv.android.xposed.callbacks.XC_LoadPackage
import dev.okhsunrog.vpnhide.generated.IfaceLists
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean

/**
 * VpnHide — hide VPN presence from apps via system_server Binder hooks.
 *
 * Hooks writeToParcel() on NetworkCapabilities, NetworkInfo, and
 * LinkProperties inside system_server. When the Binder caller is a
 * target UID, VPN-related data is stripped before serialization —
 * the app receives clean data without any in-process hooks.
 *
 * This covers all Java API detection paths:
 *   - NetworkCapabilities: hasTransport(VPN), hasCapability(NOT_VPN),
 *     getTransportTypes(), getTransportInfo(), toString()
 *   - NetworkInfo: getType(), getTypeName()
 *   - ConnectivityManager: all methods that return NetworkCapabilities,
 *     NetworkInfo, or LinkProperties over Binder
 *   - LinkProperties: getInterfaceName(), getRoutes(), getDnsServers()
 *
 * Native detection paths (getifaddrs, ioctl, /proc/net) are covered
 * by vpnhide-kmod (kernel module) or vpnhide-zygisk (in-process hooks).
 *
 * Only "System Framework" needs to be in LSPosed scope.
 */
class HookEntry : IXposedHookLoadPackage {
    private val hookInstalled = AtomicBoolean(false)

    override fun handleLoadPackage(lpparam: XC_LoadPackage.LoadPackageParam) {
        // Only hook system_server. handleLoadPackage fires multiple times
        // in system_server (once per hosted package / APEX), so we use
        // compareAndSet to install hooks exactly once.
        val inSystemServer =
            hookInstalled.get() ||
                lpparam.processName == "android" ||
                android.os.Process.myUid() == 1000

        if (!inSystemServer) return

        if (hookInstalled.compareAndSet(false, true)) {
            HookLog.install()
            HookLog.i("VpnHide: system_server detected, installing Binder hooks")
            installSystemServerHooks()
            tryHook("PackageVisibility") { PackageVisibilityHooks.install(lpparam.classLoader) }
            writeHookStatusFile()
        }
    }

    private inline fun tryHook(
        name: String,
        block: () -> Unit,
    ) {
        try {
            block()
        } catch (t: Throwable) {
            HookLog.e("VpnHide: $name hook failed: ${t::class.java.simpleName}: ${t.message}")
        }
    }

    // ------------------------------------------------------------------
    //  Helpers
    // ------------------------------------------------------------------

    private fun isVpnInterfaceName(name: String): Boolean = IfaceLists.isVpnIface(name)

    private fun sanitizeLinkProperties(copy: LinkProperties): Boolean {
        var modified = false

        val ifaceName = XposedHelpers.getObjectField(copy, "mIfaceName") as? String
        if (ifaceName != null && isVpnInterfaceName(ifaceName)) {
            XposedHelpers.setObjectField(copy, "mIfaceName", null)
            modified = true
        }

        try {
            @Suppress("UNCHECKED_CAST")
            val routesField = XposedHelpers.getObjectField(copy, "mRoutes") as? MutableList<RouteInfo>
            if (routesField != null) {
                val filtered =
                    routesField.filterNot { route ->
                        val routeIface = route.`interface`
                        routeIface != null && isVpnInterfaceName(routeIface)
                    }
                if (filtered.size != routesField.size) {
                    routesField.clear()
                    routesField.addAll(filtered)
                    modified = true
                }
            }
        } catch (t: Throwable) {
            HookLog.e("VpnHide: failed to sanitize mRoutes: ${t.message}")
        }

        try {
            @Suppress("UNCHECKED_CAST")
            val stacked = XposedHelpers.getObjectField(copy, "mStackedLinks") as? MutableMap<String, LinkProperties>
            if (stacked != null && stacked.isNotEmpty()) {
                val filtered = LinkedHashMap<String, LinkProperties>()
                for ((key, value) in stacked) {
                    val stackedCopy =
                        try {
                            val ctor = LinkProperties::class.java.getDeclaredConstructor(LinkProperties::class.java)
                            ctor.isAccessible = true
                            ctor.newInstance(value) as LinkProperties
                        } catch (_: Throwable) {
                            value
                        }
                    val stackedModified = sanitizeLinkProperties(stackedCopy)
                    val stackedIface = XposedHelpers.getObjectField(stackedCopy, "mIfaceName") as? String
                    if (stackedIface == null && stackedCopy.routes.isEmpty()) {
                        if (stackedModified || isVpnInterfaceName(key)) {
                            modified = true
                        } else {
                            filtered[key] = stackedCopy
                        }
                    } else {
                        if (stackedModified || stackedCopy !== value) modified = true
                        filtered[key] = stackedCopy
                    }
                }
                if (filtered.size != stacked.size || modified) {
                    stacked.clear()
                    stacked.putAll(filtered)
                }
            }
        } catch (t: Throwable) {
            HookLog.e("VpnHide: failed to sanitize mStackedLinks: ${t.message}")
        }

        return modified
    }

    // ==================================================================
    //  system_server hooks — per-UID Binder filtering
    // ==================================================================

    @Volatile private var systemServerTargetUids: Set<Int>? = null

    @Volatile private var targetUidsFileObserver: android.os.FileObserver? = null
    private val uidLock = Any()

    private fun loadTargetUids(): Set<Int> {
        // Fast path: already cached (volatile read)
        systemServerTargetUids?.let { return it }

        // Slow path: only one thread reads the file
        synchronized(uidLock) {
            systemServerTargetUids?.let { return it }

            val uids = mutableSetOf<Int>()

            // Read pre-resolved numeric UIDs written by vpnhide-kmod's
            // service.sh into /data/system/vpnhide_uids.txt.
            // system_server can read /data/system/ (SELinux: system_data_file).
            try {
                val file = File("/data/system/vpnhide_uids.txt")
                if (file.exists()) {
                    file.readLines().forEach { line ->
                        line.trim().toIntOrNull()?.let { uids.add(it) }
                    }
                }
            } catch (t: Throwable) {
                HookLog.e("VpnHide: failed to read UIDs: ${t.message}")
            }

            val result: Set<Int> = uids.toSet()
            if (result.isNotEmpty()) {
                HookLog.i("VpnHide: system_server loaded ${result.size} target UIDs: $result")
            }
            // Always cache (even if empty) to avoid re-reading until invalidated
            systemServerTargetUids = result
            return result
        }
    }

    private fun isTargetCaller(): Boolean {
        val uid = Binder.getCallingUid()
        return loadTargetUids().contains(uid)
    }

    private fun invalidateTargetUids() {
        systemServerTargetUids = null
    }

    private fun installSystemServerHooks() {
        tryHook("NC.writeToParcel") { hookNCWriteToParcel() }
        tryHook("NI.writeToParcel") { hookNIWriteToParcel() }
        tryHook("LP.writeToParcel") { hookLPWriteToParcel() }
        tryHook("FileObserver") { watchTargetUidsFile() }
    }

    /**
     * Write a status file so the VPN Hide app can verify hooks are active.
     * Includes boot_id to distinguish stale files from previous boots.
     */
    private fun writeHookStatusFile() {
        try {
            val bootId = File("/proc/sys/kernel/random/boot_id").readText().trim()
            val timestamp = System.currentTimeMillis() / 1000
            val version = BuildConfig.VERSION_NAME
            val content = "version=$version\nboot_id=$bootId\ntimestamp=$timestamp\n"
            val statusFile = File(HOOK_STATUS_FILE)
            statusFile.writeText(content)
            statusFile.setReadable(true, false)
            HookLog.i("VpnHide: wrote hook status file (version=$version, boot_id=$bootId)")
        } catch (t: Throwable) {
            HookLog.e("VpnHide: failed to write hook status: ${t.message}")
        }
    }

    /**
     * Watch /data/system/vpnhide_uids.txt for changes via inotify.
     * When modified (e.g. by the VPN Hide app), invalidate the
     * cached UID set so the next writeToParcel call re-reads it.
     */
    private fun watchTargetUidsFile() {
        val dir = "/data/system"
        val filename = "vpnhide_uids.txt"
        val observer =
            object : android.os.FileObserver(
                File(dir),
                CREATE or CLOSE_WRITE or MOVED_TO or MODIFY,
            ) {
                override fun onEvent(
                    event: Int,
                    path: String?,
                ) {
                    if (path == filename) {
                        HookLog.i("VpnHide: $filename changed (event=$event), invalidating UID cache")
                        systemServerTargetUids = null
                    }
                }
            }
        targetUidsFileObserver = observer
        observer.startWatching()
        HookLog.i("VpnHide: watching $dir for $filename changes (inotify)")
    }

    /**
     * Hook NetworkCapabilities.writeToParcel in system_server.
     * For target UIDs, creates a copy with VPN stripped and writes
     * the copy to the Parcel instead of the original. The original
     * object is never mutated, avoiding race conditions with
     * ConnectivityService threads.
     */
    private fun hookNCWriteToParcel() {
        val writingCopy = ThreadLocal<Boolean>()
        XposedHelpers.findAndHookMethod(
            NetworkCapabilities::class.java,
            "writeToParcel",
            android.os.Parcel::class.java,
            Integer.TYPE,
            object : XC_MethodHook() {
                override fun beforeHookedMethod(param: MethodHookParam) {
                    if (writingCopy.get() == true) return
                    val callerUid = Binder.getCallingUid()
                    val targets = loadTargetUids()
                    val isTarget = targets.contains(callerUid)
                    val nc = param.thisObject as NetworkCapabilities
                    val transportTypes = XposedHelpers.getLongField(nc, "mTransportTypes")
                    val hasVpn = (transportTypes and (1L shl TRANSPORT_VPN)) != 0L
                    // Per-request diagnostic line. Gated by the debug-logging
                    // toggle: these fire on every NC.writeToParcel inside
                    // system_server and directly name the target UIDs we hook,
                    // which is exactly what users hiding their setup want
                    // kept out of logcat.
                    HookLog.i(
                        "VpnHide-NC: uid=$callerUid target=$isTarget hasVpn=$hasVpn " +
                            "transports=0x${transportTypes.toString(16)}",
                    )
                    if (!isTarget) return

                    try {
                        val vpnBit = 1L shl TRANSPORT_VPN
                        if (transportTypes and vpnBit == 0L) return

                        val copy = NetworkCapabilities(nc)
                        XposedHelpers.setLongField(copy, "mTransportTypes", transportTypes and vpnBit.inv())
                        val caps = XposedHelpers.getLongField(copy, "mNetworkCapabilities")
                        XposedHelpers.setLongField(copy, "mNetworkCapabilities", caps or (1L shl NET_CAPABILITY_NOT_VPN))
                        try {
                            val ti = XposedHelpers.getObjectField(copy, "mTransportInfo")
                            if (ti != null && ti.javaClass.name == "android.net.VpnTransportInfo") {
                                XposedHelpers.setObjectField(copy, "mTransportInfo", null)
                            }
                        } catch (_: Throwable) {
                        }

                        val parcel = param.args[0] as android.os.Parcel
                        val flags = param.args[1] as Int
                        writingCopy.set(true)
                        try {
                            copy.writeToParcel(parcel, flags)
                        } finally {
                            writingCopy.set(false)
                        }
                        param.result = null
                        HookLog.i("VpnHide-NC: uid=$callerUid STRIPPED VPN")
                    } catch (t: Throwable) {
                        HookLog.e("VpnHide: NC.writeToParcel error: ${t.message}")
                    }
                }
            },
        )
        HookLog.i("VpnHide: hooked NetworkCapabilities.writeToParcel")
    }

    /**
     * Hook NetworkInfo.writeToParcel — disguise VPN NetworkInfo for target callers.
     * Creates a copy with type changed from VPN to WIFI, writes the copy.
     */
    @Suppress("DEPRECATION")
    private fun hookNIWriteToParcel() {
        val writingCopy = ThreadLocal<Boolean>()
        XposedHelpers.findAndHookMethod(
            NetworkInfo::class.java,
            "writeToParcel",
            android.os.Parcel::class.java,
            Integer.TYPE,
            object : XC_MethodHook() {
                override fun beforeHookedMethod(param: MethodHookParam) {
                    if (writingCopy.get() == true) return
                    val callerUid = Binder.getCallingUid()
                    val isTarget = loadTargetUids().contains(callerUid)
                    val ni = param.thisObject as NetworkInfo
                    val type = XposedHelpers.getIntField(ni, "mNetworkType")
                    val isVpn = type == TYPE_VPN
                    HookLog.i(
                        "VpnHide-NI: uid=$callerUid target=$isTarget isVpn=$isVpn type=$type",
                    )
                    if (!isTarget) return
                    try {
                        if (!isVpn) return

                        val ctor =
                            NetworkInfo::class.java.getDeclaredConstructor(
                                Integer.TYPE,
                                Integer.TYPE,
                                String::class.java,
                                String::class.java,
                            )
                        ctor.isAccessible = true
                        val copy = ctor.newInstance(TYPE_WIFI, 0, "WIFI", "") as NetworkInfo
                        XposedHelpers.setIntField(copy, "mState", XposedHelpers.getIntField(ni, "mState"))
                        XposedHelpers.setIntField(copy, "mDetailedState", XposedHelpers.getIntField(ni, "mDetailedState"))
                        XposedHelpers.setBooleanField(copy, "mIsAvailable", XposedHelpers.getBooleanField(ni, "mIsAvailable"))

                        val parcel = param.args[0] as android.os.Parcel
                        val flags = param.args[1] as Int
                        writingCopy.set(true)
                        try {
                            copy.writeToParcel(parcel, flags)
                        } finally {
                            writingCopy.set(false)
                        }
                        param.result = null
                        HookLog.i("VpnHide-NI: uid=$callerUid STRIPPED VPN (disguised as WIFI)")
                    } catch (t: Throwable) {
                        HookLog.e("VpnHide: NI.writeToParcel error: ${t.message}")
                    }
                }
            },
        )
        HookLog.i("VpnHide: hooked NetworkInfo.writeToParcel")
    }

    /**
     * Hook LinkProperties.writeToParcel — clear VPN interface name and
     * routes for target callers. Creates a copy to avoid mutating the
     * original object shared by ConnectivityService threads.
     */
    private fun hookLPWriteToParcel() {
        val writingCopy = ThreadLocal<Boolean>()
        XposedHelpers.findAndHookMethod(
            LinkProperties::class.java,
            "writeToParcel",
            android.os.Parcel::class.java,
            Integer.TYPE,
            object : XC_MethodHook() {
                override fun beforeHookedMethod(param: MethodHookParam) {
                    if (writingCopy.get() == true) return
                    val callerUid = Binder.getCallingUid()
                    val isTarget = loadTargetUids().contains(callerUid)
                    val lp = param.thisObject as LinkProperties
                    val ifname = XposedHelpers.getObjectField(lp, "mIfaceName") as? String
                    HookLog.i("VpnHide-LP: uid=$callerUid target=$isTarget ifname=$ifname")
                    if (!isTarget) return
                    try {
                        val ctor = LinkProperties::class.java.getDeclaredConstructor(LinkProperties::class.java)
                        ctor.isAccessible = true
                        val copy = ctor.newInstance(lp) as LinkProperties
                        if (!sanitizeLinkProperties(copy)) return

                        val parcel = param.args[0] as android.os.Parcel
                        val flags = param.args[1] as Int
                        writingCopy.set(true)
                        try {
                            copy.writeToParcel(parcel, flags)
                        } finally {
                            writingCopy.set(false)
                        }
                        param.result = null
                        HookLog.i("VpnHide-LP: uid=$callerUid STRIPPED VPN (ifname was $ifname)")
                    } catch (t: Throwable) {
                        HookLog.e("VpnHide: LP.writeToParcel error: ${t.message}")
                    }
                }
            },
        )
        HookLog.i("VpnHide: hooked LinkProperties.writeToParcel")
    }

    companion object {
        private const val TRANSPORT_VPN = 4
        private const val NET_CAPABILITY_NOT_VPN = 15
        private const val TYPE_VPN = 17
        private const val TYPE_WIFI = 1
        const val HOOK_STATUS_FILE = "/data/system/vpnhide_hook_active"
    }
}
