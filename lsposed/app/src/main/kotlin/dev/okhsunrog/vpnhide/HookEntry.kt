package dev.okhsunrog.vpnhide

import android.net.LinkProperties
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import android.os.Binder
import de.robv.android.xposed.IXposedHookLoadPackage
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers
import de.robv.android.xposed.callbacks.XC_LoadPackage
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
            XposedBridge.log("VpnHide: system_server detected, installing Binder hooks")
            installSystemServerHooks(lpparam.classLoader)
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
            XposedBridge.log("VpnHide: $name hook failed: ${t::class.java.simpleName}: ${t.message}")
        }
    }

    // ------------------------------------------------------------------
    //  Helpers — sanitise logic lives in NetworkSanitizers so both the
    //  legacy writeToParcel hooks below and the new ConnectivityService
    //  getter hooks share one implementation.
    // ------------------------------------------------------------------

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
                XposedBridge.log("VpnHide: failed to read UIDs: ${t.message}")
            }

            val result: Set<Int> = uids.toSet()
            if (result.isNotEmpty()) {
                XposedBridge.log("VpnHide: system_server loaded ${result.size} target UIDs: $result")
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

    private fun installSystemServerHooks(classLoader: ClassLoader) {
        tryHook("NC.writeToParcel") { hookNCWriteToParcel() }
        tryHook("NI.writeToParcel") { hookNIWriteToParcel() }
        tryHook("LP.writeToParcel") { hookLPWriteToParcel() }
        tryHook("ConnectivityService getters") {
            ConnectivityServiceHooks.install(
                classLoader = classLoader,
                isTargetCaller = ::isTargetCaller,
                sanitizeNetworkCapabilities = NetworkSanitizers::sanitizeNetworkCapabilities,
                sanitizeNetworkInfo = NetworkSanitizers::sanitizeNetworkInfo,
                sanitizeLinkProperties = NetworkSanitizers::sanitizeLinkProperties,
            )
        }
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
            XposedBridge.log("VpnHide: wrote hook status file (version=$version, boot_id=$bootId)")
        } catch (t: Throwable) {
            XposedBridge.log("VpnHide: failed to write hook status: ${t.message}")
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
                        XposedBridge.log("VpnHide: $filename changed (event=$event), invalidating UID cache")
                        systemServerTargetUids = null
                    }
                }
            }
        targetUidsFileObserver = observer
        observer.startWatching()
        XposedBridge.log("VpnHide: watching $dir for $filename changes (inotify)")
    }

    /**
     * Hook NetworkCapabilities.writeToParcel — defensive fallback for
     * callback/broadcast paths the ConnectivityService getter hooks may
     * not cover. For target UIDs, writes a VPN-stripped copy to the
     * Parcel instead of the original; the original object is never
     * mutated.
     *
     * Slated for removal once server-side hooks prove sufficient.
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
                    if (!isTargetCaller()) return

                    val nc = param.thisObject as NetworkCapabilities
                    try {
                        val sanitized = NetworkSanitizers.sanitizeNetworkCapabilities(nc) ?: return
                        val parcel = param.args[0] as android.os.Parcel
                        val flags = param.args[1] as Int
                        writingCopy.set(true)
                        try {
                            sanitized.writeToParcel(parcel, flags)
                        } finally {
                            writingCopy.set(false)
                        }
                        param.result = null
                    } catch (t: Throwable) {
                        XposedBridge.log("VpnHide: NC.writeToParcel error: ${t.message}")
                    }
                }
            },
        )
        XposedBridge.log("VpnHide: hooked NetworkCapabilities.writeToParcel")
    }

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
                    if (!isTargetCaller()) return
                    val ni = param.thisObject as NetworkInfo
                    try {
                        val sanitized = NetworkSanitizers.sanitizeNetworkInfo(ni) ?: return
                        val parcel = param.args[0] as android.os.Parcel
                        val flags = param.args[1] as Int
                        writingCopy.set(true)
                        try {
                            sanitized.writeToParcel(parcel, flags)
                        } finally {
                            writingCopy.set(false)
                        }
                        param.result = null
                    } catch (t: Throwable) {
                        XposedBridge.log("VpnHide: NI.writeToParcel error: ${t.message}")
                    }
                }
            },
        )
        XposedBridge.log("VpnHide: hooked NetworkInfo.writeToParcel")
    }

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
                    if (!isTargetCaller()) return
                    val lp = param.thisObject as LinkProperties
                    try {
                        val sanitized = NetworkSanitizers.sanitizeLinkProperties(lp) ?: return
                        val parcel = param.args[0] as android.os.Parcel
                        val flags = param.args[1] as Int
                        writingCopy.set(true)
                        try {
                            sanitized.writeToParcel(parcel, flags)
                        } finally {
                            writingCopy.set(false)
                        }
                        param.result = null
                    } catch (t: Throwable) {
                        XposedBridge.log("VpnHide: LP.writeToParcel error: ${t.message}")
                    }
                }
            },
        )
        XposedBridge.log("VpnHide: hooked LinkProperties.writeToParcel")
    }

    companion object {
        const val HOOK_STATUS_FILE = "/data/system/vpnhide_hook_active"
    }
}
