package dev.okhsunrog.vpnhide

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import android.net.RouteInfo
import android.os.Binder
import android.os.Build
import android.os.Bundle
import de.robv.android.xposed.IXposedHookLoadPackage
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers
import de.robv.android.xposed.callbacks.XC_LoadPackage
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.generated.IfaceLists
import java.util.concurrent.atomic.AtomicBoolean
import java.lang.reflect.Array as JavaArray

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
 *
 * Single Xposed entry point for system_server hook wiring; splitting the hook
 * installer is a separate refactor from adding telemetry.
 *
 * Deprecated legacy connectivity APIs are still active detection surfaces.
 */
@Suppress("DEPRECATION", "LargeClass")
class HookEntry : IXposedHookLoadPackage {
    private val hookInstalled = AtomicBoolean(false)

    // Guards installing the ConnectivityService callback hook exactly once —
    // it can be triggered from either the direct lookup or the addService catch.
    private val connectivityHooked = AtomicBoolean(false)

    // During a push callback (registerNetworkCallback dispatch), the
    // writeToParcel hooks run under system_server's identity, so
    // Binder.getCallingUid() is 1000 — not the recipient app. hookConnectivity-
    // Service stashes the real recipient UID here so those hooks sanitize the
    // pushed data exactly like a synchronous call. See issue #70 (VTB and other
    // apps that detect VPN only via registerDefaultNetworkCallback).
    private val currentCallbackUid = ThreadLocal<Int>()
    private val bypassConnectivitySanitize = ThreadLocal<Boolean>()

    @Volatile private var connectivityServiceInstance: Any? = null

    private fun effectiveCallerUid(): Int {
        val uid = Binder.getCallingUid()
        return if (uid == SYSTEM_UID) currentCallbackUid.get() ?: uid else uid
    }

    private fun isTargetCallerOrUid(
        hook: HookIds.Hook,
        uid: Int? = null,
    ): Boolean =
        SystemServerConfigCache.isHookEnabledForUid(effectiveCallerUid(), hook) ||
            (uid != null && SystemServerConfigCache.isHookEnabledForUid(uid, hook))

    private fun rememberConnectivityService(instance: Any?) {
        if (instance != null) connectivityServiceInstance = instance
    }

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
            val hookInstall = installSystemServerHooks()
            var installedMask = hookInstall.installedHookMask
            val installFailures = hookInstall.installFailures.toMutableList()
            if (tryHook("PackageVisibility", installFailures) { PackageVisibilityHooks.install(lpparam.classLoader) }) {
                installedMask = installedMask or hookBit(HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY)
            }
            if (tryHook("ConnectivityService", installFailures) { installConnectivityServiceHook(lpparam.classLoader) }) {
                installedMask =
                    installedMask or
                    hookBit(HookIds.Hook.LSPOSED_CONNECTIVITY_RESULT) or
                    hookBit(HookIds.Hook.LSPOSED_CONNECTIVITY_CALLBACK) or
                    hookBit(HookIds.Hook.LSPOSED_CONNECTIVITY_NETWORK)
            }
            LsposedStats.setStatus(installedMask, hookInstall.brokenFields, installFailures)
        }
    }

    private inline fun tryHook(
        name: String,
        failures: MutableList<String>? = null,
        block: () -> Unit,
    ): Boolean =
        try {
            block()
            true
        } catch (t: Throwable) {
            HookLog.e("VpnHide: $name hook failed: ${t::class.java.simpleName}: ${t.message}")
            failures?.add(formatInstallFailure(name, t))
            false
        }

    private fun formatInstallFailure(
        name: String,
        t: Throwable,
    ): String = "$name: ${t::class.java.simpleName}: ${t.message.orEmpty()}"

    // ------------------------------------------------------------------
    //  Helpers
    // ------------------------------------------------------------------

    private fun isVpnInterfaceName(name: String): Boolean = IfaceLists.isVpnIface(name)

    private fun hookBit(hook: HookIds.Hook): Int = 1 shl hook.id

    // Recursively sanitizes mIfaceName + mRoutes + nested mStackedLinks; the
    // length and nesting are inherent to walking that object graph by reflection.
    // Still private-field-based (unlike sanitizeNetworkCapabilities, which moved
    // to public mutators after Android 17 renamed NC's private fields). LP's
    // fields are stable so far; if a future Android renames mIfaceName/mRoutes/
    // mStackedLinks, migrate this to the public LinkProperties API
    // (setInterfaceName(null) / setLinkAddresses / setRoutes / setDnsServers)
    // the same way NC was done, and drop LP from the install-time smoke-check.
    private fun sanitizeLinkProperties(copy: LinkProperties): Boolean {
        var modified = false

        val ifaceName = XposedHelpers.getObjectField(copy, "mIfaceName") as? String
        val isVpnLp = ifaceName != null && isVpnInterfaceName(ifaceName)
        if (isVpnLp) {
            XposedHelpers.setObjectField(copy, "mIfaceName", null)
            modified = true
        }

        // mLinkAddresses (the tunnel's assigned IP) and mDnses (the VPN's DNS
        // servers) carry no interface tag, so they can only be scrubbed when the
        // whole LinkProperties is a VPN one. Leaving them let an app read the
        // VPN's tunnel address / DNS straight back via getLinkAddresses() /
        // getDnsServers(). Clear both for a VPN LP (the routes/iface above are
        // already handled).
        if (isVpnLp) {
            if (clearLinkPropertyList(copy, "mLinkAddresses")) modified = true
            if (clearLinkPropertyList(copy, "mDnses")) modified = true
        }

        if (sanitizeLinkRoutes(copy)) modified = true
        if (sanitizeStackedLinks(copy)) modified = true

        return modified
    }

    /** Remove routes whose interface is a VPN tunnel. Returns true if any went. */
    private fun sanitizeLinkRoutes(copy: LinkProperties): Boolean {
        try {
            @Suppress("UNCHECKED_CAST")
            val routesField = XposedHelpers.getObjectField(copy, "mRoutes") as? MutableList<RouteInfo> ?: return false
            val filtered =
                routesField.filterNot { route ->
                    val routeIface = route.`interface`
                    routeIface != null && isVpnInterfaceName(routeIface)
                }
            if (filtered.size != routesField.size) {
                routesField.clear()
                routesField.addAll(filtered)
                return true
            }
        } catch (t: Throwable) {
            HookLog.e("VpnHide: failed to sanitize mRoutes: ${t.message}")
        }
        return false
    }

    /** Recursively sanitize stacked LinkProperties, dropping ones that become
     *  empty VPN tunnels. Returns true if anything changed. */
    @Suppress("NestedBlockDepth") // try-inside-for-inside-if-else over the stacked-LP map is structurally unavoidable
    private fun sanitizeStackedLinks(copy: LinkProperties): Boolean {
        var modified = false
        try {
            @Suppress("UNCHECKED_CAST")
            val stacked = XposedHelpers.getObjectField(copy, "mStackedLinks") as? MutableMap<String, LinkProperties>
            if (stacked != null && stacked.isNotEmpty()) {
                val filtered = LinkedHashMap<String, LinkProperties>()
                for ((key, value) in stacked) {
                    val stackedCopy = cloneLinkProperties(value)
                    val stackedModified = sanitizeLinkProperties(stackedCopy)
                    val stackedIface = XposedHelpers.getObjectField(stackedCopy, "mIfaceName") as? String
                    if (stackedIface == null && stackedCopy.routes.isEmpty()) {
                        if (stackedModified || isVpnInterfaceName(key)) {
                            modified = true
                        } else {
                            filtered[key] = stackedCopy
                        }
                    } else {
                        if (stackedModified) modified = true
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

    /** Deep-copy a LinkProperties via its copy constructor, falling back to the
     *  original on any reflection failure. */
    private fun cloneLinkProperties(value: LinkProperties): LinkProperties =
        try {
            val ctor = LinkProperties::class.java.getDeclaredConstructor(LinkProperties::class.java)
            ctor.isAccessible = true
            ctor.newInstance(value) as LinkProperties
        } catch (_: Throwable) {
            value
        }

    /** Clear a `MutableList` field on a LinkProperties by reflection; returns
     *  true if it had entries that were removed. */
    private fun clearLinkPropertyList(
        copy: LinkProperties,
        field: String,
    ): Boolean =
        try {
            val list = XposedHelpers.getObjectField(copy, field) as? MutableList<*>
            if (!list.isNullOrEmpty()) {
                list.clear()
                true
            } else {
                false
            }
        } catch (t: Throwable) {
            HookLog.e("VpnHide: failed to clear $field: ${t.message}")
            false
        }

    private fun sanitizeNetworkCapabilities(copy: NetworkCapabilities): Boolean {
        val hasVpnTransport = copy.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
        val hasVpnInfo = copy.transportInfo?.javaClass?.name == "android.net.VpnTransportInfo"

        if (!hasVpnTransport && !hasVpnInfo) return false

        if (hasVpnTransport) {
            XposedHelpers.callMethod(copy, "removeTransportType", NetworkCapabilities.TRANSPORT_VPN)
        }
        XposedHelpers.callMethod(copy, "addCapability", NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
        if (hasVpnInfo) clearTransportInfo(copy)

        return true
    }

    private fun clearTransportInfo(copy: NetworkCapabilities) {
        val transportInfoClass = Class.forName("android.net.TransportInfo")
        XposedHelpers.callMethod(
            copy,
            "setTransportInfo",
            arrayOf(transportInfoClass),
            *arrayOfNulls<Any>(1),
        )
    }

    private inline fun <T> withConnectivitySanitizeBypassed(block: () -> T): T {
        val wasBypassed = bypassConnectivitySanitize.get() == true
        bypassConnectivitySanitize.set(true)
        return try {
            block()
        } finally {
            if (wasBypassed) {
                bypassConnectivitySanitize.set(true)
            } else {
                bypassConnectivitySanitize.remove()
            }
        }
    }

    private inline fun <T> withClearedCallingIdentity(block: () -> T): T {
        val token = Binder.clearCallingIdentity()
        return try {
            block()
        } finally {
            Binder.restoreCallingIdentity(token)
        }
    }

    private fun rawNetworkCapabilities(
        cs: Any,
        network: Network,
    ): NetworkCapabilities? =
        withClearedCallingIdentity {
            withConnectivitySanitizeBypassed {
                callNetworkCapabilities(cs, network)
            }
        }

    private fun callNetworkCapabilities(
        cs: Any,
        network: Network,
    ): NetworkCapabilities? {
        val typedArgs = arrayOf<Any?>(network, "android", null)
        try {
            return XposedHelpers.callMethod(
                cs,
                "getNetworkCapabilities",
                arrayOf(Network::class.java, String::class.java, String::class.java),
                *typedArgs,
            ) as? NetworkCapabilities
        } catch (_: Throwable) {
        }
        return try {
            XposedHelpers.callMethod(cs, "getNetworkCapabilities", network) as? NetworkCapabilities
        } catch (_: Throwable) {
            null
        }
    }

    private fun rawAllNetworks(cs: Any): List<Network> =
        withClearedCallingIdentity {
            withConnectivitySanitizeBypassed {
                ((XposedHelpers.callMethod(cs, "getAllNetworks") as? Array<*>) ?: emptyArray<Any>())
                    .filterIsInstance<Network>()
            }
        }

    private fun isVpnNetwork(
        cs: Any,
        network: Network,
    ): Boolean = rawNetworkCapabilities(cs, network)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true

    private fun hasPhysicalTransport(caps: NetworkCapabilities): Boolean {
        val transportTypes =
            try {
                XposedHelpers.callMethod(caps, "getTransportTypes") as? IntArray
            } catch (_: Throwable) {
                null
            }
        if (transportTypes != null) {
            return transportTypes.any { it != NetworkCapabilities.TRANSPORT_VPN }
        }

        return caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
            caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) ||
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) ||
            caps.hasTransport(NetworkCapabilities.TRANSPORT_BLUETOOTH)
    }

    private fun physicalNetworkScore(caps: NetworkCapabilities): Int {
        var score = 0
        if (caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) score += 40
        if (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) score += 30
        if (caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) score += 20
        if (caps.hasTransport(NetworkCapabilities.TRANSPORT_BLUETOOTH)) score += 10
        if (hasPhysicalTransport(caps)) score += 1
        if (caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)) score += 4
        if (caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)) score += 8
        return score
    }

    // Physical replacement follows the recommended split-tunnel model: target
    // apps see a non-VPN Network and may bind sockets to it. That is right for
    // apps kept outside the VPN, but it can bypass the VPN for apps that must
    // keep traffic inside the tunnel while hiding VPN state.
    // TODO: add a VPN-preserving concealment mode for that use case
    // (tracked by GitHub issue 130).
    private fun findPhysicalNetwork(cs: Any): Network? {
        val scored =
            rawAllNetworks(cs).mapNotNull { network ->
                val caps = rawNetworkCapabilities(cs, network) ?: return@mapNotNull null
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) || !hasPhysicalTransport(caps)) {
                    null
                } else {
                    network to physicalNetworkScore(caps)
                }
            }
        return scored.maxByOrNull { it.second }?.first
    }

    private fun sanitizedNetworkCapabilities(nc: NetworkCapabilities): NetworkCapabilities {
        val copy = NetworkCapabilities(nc)
        return if (sanitizeNetworkCapabilities(copy)) copy else nc
    }

    private fun sanitizedLinkProperties(lp: LinkProperties): LinkProperties {
        val ctor = LinkProperties::class.java.getDeclaredConstructor(LinkProperties::class.java)
        ctor.isAccessible = true
        val copy = ctor.newInstance(lp) as LinkProperties
        return if (sanitizeLinkProperties(copy)) copy else lp
    }

    // NetworkInfo is legacy/deprecated and its fields have been stable, so this
    // still copies mNetworkType/mState/mDetailedState/mIsAvailable by reflection
    // (guarded by the install-time smoke-check). If a future Android renames
    // them — as Android 17 did for NetworkCapabilities — migrate to the public
    // NetworkInfo API (setDetailedState(...) sets state+detailedState; there's
    // no public type setter, so reconstruct via the public ctor like here) the
    // same way NC was moved to public mutators, and drop NI from the smoke-check.
    @Suppress("DEPRECATION")
    private fun sanitizedNetworkInfo(ni: NetworkInfo): NetworkInfo {
        val type = XposedHelpers.getIntField(ni, "mNetworkType")
        if (type != ConnectivityManager.TYPE_VPN) return ni

        val ctor =
            NetworkInfo::class.java.getDeclaredConstructor(
                Integer.TYPE,
                Integer.TYPE,
                String::class.java,
                String::class.java,
            )
        ctor.isAccessible = true
        val copy = ctor.newInstance(ConnectivityManager.TYPE_WIFI, 0, "WIFI", "") as NetworkInfo
        XposedHelpers.setObjectField(copy, "mState", XposedHelpers.getObjectField(ni, "mState"))
        XposedHelpers.setObjectField(copy, "mDetailedState", XposedHelpers.getObjectField(ni, "mDetailedState"))
        XposedHelpers.setBooleanField(copy, "mIsAvailable", XposedHelpers.getBooleanField(ni, "mIsAvailable"))
        return copy
    }

    private fun sanitizedValue(value: Any?): Any? =
        when (value) {
            is NetworkCapabilities -> sanitizedNetworkCapabilities(value)
            is LinkProperties -> sanitizedLinkProperties(value)
            is NetworkInfo -> sanitizedNetworkInfo(value)
            is Array<*> -> sanitizedArray(value)
            else -> value
        }

    private fun sanitizedArray(values: Array<*>): Any {
        val componentType = values.javaClass.componentType ?: return values
        if (
            componentType != NetworkCapabilities::class.java &&
            componentType != NetworkInfo::class.java
        ) {
            return values
        }

        val copy = JavaArray.newInstance(componentType, values.size)
        var modified = false
        for (i in values.indices) {
            val sanitized = sanitizedValue(values[i])
            if (sanitized !== values[i]) modified = true
            JavaArray.set(copy, i, sanitized)
        }
        return if (modified) copy else values
    }

    private fun sanitizeMethodResult(
        param: XC_MethodHook.MethodHookParam,
        explicitUid: Int? = null,
    ) {
        if (bypassConnectivitySanitize.get() == true) return
        if (!isTargetCallerOrUid(HookIds.Hook.LSPOSED_CONNECTIVITY_RESULT, explicitUid)) return
        try {
            val original = param.result
            val sanitized = sanitizedValue(original)
            if (sanitized !== original) {
                param.result = sanitized
                LsposedStats.record(explicitUid ?: effectiveCallerUid(), HookIds.Hook.LSPOSED_CONNECTIVITY_RESULT)
            }
        } catch (t: Throwable) {
            HookLog.e("VpnHide: ConnectivityService result sanitize error: ${t.message}")
        }
    }

    @Suppress("DEPRECATION")
    private fun sanitizeCallbackBundle(bundle: Bundle): Boolean {
        var modified = false
        try {
            val nc = bundle.getParcelable(NetworkCapabilities::class.java.simpleName) as? NetworkCapabilities
            if (nc != null) {
                val sanitized = sanitizedNetworkCapabilities(nc)
                if (sanitized !== nc) {
                    bundle.putParcelable(NetworkCapabilities::class.java.simpleName, sanitized)
                    modified = true
                }
            }
            val lp = bundle.getParcelable(LinkProperties::class.java.simpleName) as? LinkProperties
            if (lp != null) {
                val sanitized = sanitizedLinkProperties(lp)
                if (sanitized !== lp) {
                    bundle.putParcelable(LinkProperties::class.java.simpleName, sanitized)
                    modified = true
                }
            }
        } catch (t: Throwable) {
            HookLog.e("VpnHide: callback bundle sanitize error: ${t.message}")
        }
        return modified
    }

    // ==================================================================
    //  system_server hooks — per-UID Binder filtering
    // ==================================================================

    @Volatile private var canonicalConfigFileObserver: android.os.FileObserver? = null

    private fun isTargetUid(
        uid: Int,
        hook: HookIds.Hook,
    ): Boolean = SystemServerConfigCache.isHookEnabledForUid(uid, hook)

    // Smoke-check at install time: every private AOSP field/ctor we touch
    // by reflection in the writeToParcel hooks. Returns the keys that
    // failed (missing or wrong-typed). Empty list = all good.
    //
    // Per-hook gates below skip installing a hook entirely when its
    // critical reflection broke — silent fail-open is preferable to
    // throwing NoSuchFieldError on every writeToParcel call (system_server
    // gets that on every NetworkCapabilities IPC, target or not). The
    // dashboard surfaces the broken_fields list as a red error so the
    // user can see and report the AOSP drift.
    private data class HookInstallResult(
        val brokenFields: List<String>,
        val installedHookMask: Int,
        val installFailures: List<String>,
    )

    private fun installSystemServerHooks(): HookInstallResult {
        val brokenFields = runReflectionSmokeCheck()
        if (brokenFields.isNotEmpty()) {
            HookLog.e("VpnHide: reflection smoke-check found broken keys: $brokenFields")
        }
        var installedHookMask = 0
        val installFailures = mutableListOf<String>()

        // Match a probe key against either an exact entry in `broken` or
        // an entry with a `:type=...` suffix (wrong-typed field).
        fun anyBroken(critical: Set<String>): Boolean = brokenFields.any { it.substringBefore(':') in critical }

        // LP: mIfaceName + copy ctor are critical. mRoutes / mStackedLinks
        // are non-critical — the existing inner try/catch in
        // sanitizeLinkProperties already lets the rest of the sanitizer
        // proceed when those are absent.
        if (anyBroken(LP_CRITICAL_KEYS)) {
            HookLog.e("VpnHide: LP.writeToParcel hook SKIPPED — critical reflection broken")
            installFailures += "LP.writeToParcel: skipped critical reflection broken: ${brokenFields.joinToString(",")}"
        } else {
            if (tryHook("LP.writeToParcel", installFailures) { hookLPWriteToParcel() }) {
                installedHookMask = installedHookMask or hookBit(HookIds.Hook.LSPOSED_LINK_PROPERTIES)
            }
        }

        // NC uses public NetworkCapabilities mutators now, so private AOSP
        // field drift must not disable this hook.
        if (tryHook("NC.writeToParcel", installFailures) { hookNCWriteToParcel() }) {
            installedHookMask = installedHookMask or hookBit(HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES)
        }

        // NI: every field + ctor is critical — the hook body has no
        // inner try/catch around the per-field setIntField/setBooleanField
        // calls, so any rename would fail-open per call with logcat spam.
        if (anyBroken(NI_CRITICAL_KEYS)) {
            HookLog.e("VpnHide: NI.writeToParcel hook SKIPPED — critical reflection broken")
            installFailures += "NI.writeToParcel: skipped critical reflection broken: ${brokenFields.joinToString(",")}"
        } else {
            if (tryHook("NI.writeToParcel", installFailures) { hookNIWriteToParcel() }) {
                installedHookMask = installedHookMask or hookBit(HookIds.Hook.LSPOSED_NETWORK_INFO)
            }
        }
        if (tryHook("Network.writeToParcel", installFailures) { hookNetworkWriteToParcel() }) {
            installedHookMask = installedHookMask or hookBit(HookIds.Hook.LSPOSED_NETWORK)
        }

        tryHook("FileObserver", installFailures) { watchCanonicalConfigFile() }
        return HookInstallResult(brokenFields, installedHookMask, installFailures)
    }

    private data class FieldProbe(
        val key: String,
        val clazz: Class<*>,
        val name: String,
        // If the device's SDK is below this, the probe is skipped entirely
        // (not "found", not "broken" — not applicable). Used for fields
        // introduced after our minSdk floor (e.g. mTransportInfo at API 29).
        // Listed before `typeCheck` so the latter stays the last parameter
        // — that lets call sites use trailing-lambda syntax for the probe
        // without having to name `typeCheck =` every time.
        val minSdk: Int = 0,
        // Field-type compatibility predicate. For collections we use
        // isAssignableFrom() so AOSP swapping ArrayList → LinkedList stays OK.
        val typeCheck: (Class<*>) -> Boolean,
    )

    private data class CtorProbe(
        val key: String,
        val clazz: Class<*>,
        val params: Array<Class<*>>,
    )

    private fun runReflectionSmokeCheck(): List<String> {
        val broken = mutableListOf<String>()
        for (probe in FIELD_PROBES) {
            if (Build.VERSION.SDK_INT < probe.minSdk) continue
            val field =
                try {
                    XposedHelpers.findField(probe.clazz, probe.name)
                } catch (_: NoSuchFieldError) {
                    broken += probe.key
                    continue
                }
            if (!probe.typeCheck(field.type)) {
                // Suffix carries the actual type to help debug AOSP-drift
                // bug reports without rebuilding/instrumenting the device.
                broken += "${probe.key}:type=${field.type.name}"
            }
        }
        for (probe in CTOR_PROBES) {
            try {
                probe.clazz.getDeclaredConstructor(*probe.params)
            } catch (_: NoSuchMethodException) {
                broken += probe.key
            }
        }
        return broken
    }

    /**
     * Watch the canonical config for changes via inotify. The cache also
     * fingerprints `/data/system/packages.list` periodically, so package
     * reinstall UID/appId changes are picked up without PackageManager calls.
     */
    private fun watchCanonicalConfigFile() {
        val filename = "vpnhide_config.json"
        canonicalConfigFileObserver =
            watchSystemDataDir { path ->
                if (path == filename) {
                    HookLog.i("VpnHide: $filename changed, invalidating config cache")
                    SystemServerConfigCache.invalidate()
                }
            }
        HookLog.i("VpnHide: watching /data/system for $filename changes (inotify)")
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
                    val callerUid = effectiveCallerUid()
                    val isTarget = isTargetUid(callerUid, HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES)
                    val nc = param.thisObject as NetworkCapabilities
                    val hasVpn = nc.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
                    // Per-request diagnostic line. Gated by the debug-logging
                    // toggle: these fire on every NC.writeToParcel inside
                    // system_server and directly name the target UIDs we hook,
                    // which is exactly what users hiding their setup want
                    // kept out of logcat.
                    HookLog.i(
                        "VpnHide-NC: uid=$callerUid target=$isTarget hasVpn=$hasVpn",
                    )
                    if (!isTarget) return

                    try {
                        val copy = NetworkCapabilities(nc)
                        if (!sanitizeNetworkCapabilities(copy)) return

                        val parcel = param.args[0] as android.os.Parcel
                        val flags = param.args[1] as Int
                        writingCopy.set(true)
                        try {
                            copy.writeToParcel(parcel, flags)
                        } finally {
                            writingCopy.set(false)
                        }
                        param.result = null
                        LsposedStats.record(callerUid, HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES)
                        HookLog.i("VpnHide-NC: uid=$callerUid STRIPPED VPN")
                    } catch (t: Throwable) {
                        HookLog.e("VpnHide: NC.writeToParcel error: ${t.message}")
                    }
                }
            },
        )
        HookLog.i("VpnHide: hooked NetworkCapabilities.writeToParcel")
    }

    private fun hookNetworkWriteToParcel() {
        val writingCopy = ThreadLocal<Boolean>()
        XposedHelpers.findAndHookMethod(
            Network::class.java,
            "writeToParcel",
            android.os.Parcel::class.java,
            Integer.TYPE,
            object : XC_MethodHook() {
                override fun beforeHookedMethod(param: MethodHookParam) {
                    if (writingCopy.get() == true || !isTargetCallerOrUid(HookIds.Hook.LSPOSED_NETWORK)) return
                    val cs = connectivityServiceInstance ?: return
                    val network = param.thisObject as Network
                    try {
                        if (!isVpnNetwork(cs, network)) return
                        val replacement = findPhysicalNetwork(cs) ?: return
                        val parcel = param.args[0] as android.os.Parcel
                        val flags = param.args[1] as Int
                        writingCopy.set(true)
                        try {
                            replacement.writeToParcel(parcel, flags)
                        } finally {
                            writingCopy.set(false)
                        }
                        param.result = null
                        LsposedStats.record(effectiveCallerUid(), HookIds.Hook.LSPOSED_NETWORK)
                        HookLog.i("VpnHide: replaced VPN Network parcel for uid=${effectiveCallerUid()}")
                    } catch (t: Throwable) {
                        HookLog.e("VpnHide: Network.writeToParcel error: ${t.message}")
                    }
                }
            },
        )
        HookLog.i("VpnHide: hooked Network.writeToParcel")
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
                    val callerUid = effectiveCallerUid()
                    val isTarget = isTargetUid(callerUid, HookIds.Hook.LSPOSED_NETWORK_INFO)
                    val ni = param.thisObject as NetworkInfo
                    val type = XposedHelpers.getIntField(ni, "mNetworkType")
                    val isVpn = type == ConnectivityManager.TYPE_VPN
                    HookLog.i(
                        "VpnHide-NI: uid=$callerUid target=$isTarget isVpn=$isVpn type=$type",
                    )
                    if (!isTarget) return
                    try {
                        if (!isVpn) return
                        val copy = sanitizedNetworkInfo(ni)

                        val parcel = param.args[0] as android.os.Parcel
                        val flags = param.args[1] as Int
                        writingCopy.set(true)
                        try {
                            copy.writeToParcel(parcel, flags)
                        } finally {
                            writingCopy.set(false)
                        }
                        param.result = null
                        LsposedStats.record(callerUid, HookIds.Hook.LSPOSED_NETWORK_INFO)
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
                    val callerUid = effectiveCallerUid()
                    val isTarget = isTargetUid(callerUid, HookIds.Hook.LSPOSED_LINK_PROPERTIES)
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
                        LsposedStats.record(callerUid, HookIds.Hook.LSPOSED_LINK_PROPERTIES)
                        HookLog.i("VpnHide-LP: uid=$callerUid STRIPPED VPN (ifname was $ifname)")
                    } catch (t: Throwable) {
                        HookLog.e("VpnHide: LP.writeToParcel error: ${t.message}")
                    }
                }
            },
        )
        HookLog.i("VpnHide: hooked LinkProperties.writeToParcel")
    }

    /**
     * Install the ConnectivityService callback hook once the service is up.
     *
     * On Android 13+ ConnectivityService ships in the Connectivity APEX and is
     * loaded by a classloader the system_server boot classloader can't resolve —
     * findClass(...) on [bootClassLoader] throws ClassNotFound, so the hook never
     * installs and push callbacks leak (issue #70). The reliable classloader is
     * the one that loaded the registered "connectivity" binder, so we take it
     * from there. The binder isn't registered yet when hooks install at early
     * boot, so we catch ServiceManager.addService("connectivity", ...) — and also
     * try a direct lookup first to cover a late module load. Works on both the
     * APEX (A13+) and in-boot-classpath (A12-) layouts.
     */
    private fun installConnectivityServiceHook(bootClassLoader: ClassLoader) {
        hookConnectivityServiceIfPossible(bootClassLoader)

        val serviceManager = XposedHelpers.findClass("android.os.ServiceManager", bootClassLoader)
        (XposedHelpers.callStaticMethod(serviceManager, "getService", "connectivity") as? android.os.IBinder)
            ?.let { hookConnectivityFromBinder(it) }
        XposedBridge.hookAllMethods(
            serviceManager,
            "addService",
            object : XC_MethodHook() {
                override fun beforeHookedMethod(param: MethodHookParam) {
                    if (param.args.getOrNull(0) != "connectivity") return
                    (param.args.getOrNull(1) as? android.os.IBinder)?.let { hookConnectivityFromBinder(it) }
                }
            },
        )
    }

    private fun hookConnectivityFromBinder(binder: android.os.IBinder) {
        val classLoader =
            binder.javaClass.classLoader ?: run {
                HookLog.i("VpnHide: connectivity binder has no classloader; waiting for direct classloader")
                return
            }
        hookConnectivityServiceIfPossible(classLoader)
    }

    private fun hookConnectivityServiceIfPossible(classLoader: ClassLoader) {
        if (!connectivityHooked.compareAndSet(false, true)) return
        try {
            hookConnectivityService(classLoader)
        } catch (t: Throwable) {
            connectivityHooked.set(false)
            HookLog.i("VpnHide: ConnectivityService class not ready on ${classLoader.javaClass.name}: ${t.message}")
        }
    }

    private fun findConnectivityServiceClass(classLoader: ClassLoader): Class<*> =
        try {
            // Android 14+ ships ConnectivityService in the repackaged
            // Connectivity APEX namespace; older releases use the original.
            XposedHelpers.findClass(
                "android.net.connectivity.com.android.server.ConnectivityService",
                classLoader,
            )
        } catch (_: Throwable) {
            XposedHelpers.findClass("com.android.server.ConnectivityService", classLoader)
        }

    /**
     * Hook the two ConnectivityService dispatch points that *push* network state
     * to apps: callCallbackForRequest (registerNetworkCallback with a callback
     * object) and sendPendingIntentForRequest (registerNetworkCallback with a
     * PendingIntent). On both, the writeToParcel hooks would see
     * getCallingUid()==1000 instead of the recipient app and skip sanitizing, so
     * we stash the recipient UID in currentCallbackUid for the dispatch's
     * duration. If the app explicitly requested a VPN network, drop the dispatch
     * entirely — don't reveal a VPN exists. Fixes apps (e.g. VTB, issue #70) that
     * detect VPN only via callbacks.
     */
    private fun hookConnectivityService(classLoader: ClassLoader) {
        val csClass = findConnectivityServiceClass(classLoader)
        tryHook("ConnectivityService constructors") {
            XposedBridge.hookAllConstructors(
                csClass,
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        rememberConnectivityService(param.thisObject)
                    }
                },
            )
        }
        installConnectivityServiceResultHooks(csClass)
        installConnectivityServiceNetworkHooks(csClass)

        // Both methods take the NetworkRequestInfo as their first arg, so the
        // same handler covers the callback-object and PendingIntent paths.
        val dispatchHook =
            object : XC_MethodHook() {
                override fun beforeHookedMethod(param: MethodHookParam) {
                    val nri = param.args.firstOrNull() ?: return
                    rememberConnectivityService(param.thisObject)
                    val uid = extractRecipientUid(nri)
                    if (uid < 0 || !isTargetUid(uid, HookIds.Hook.LSPOSED_CONNECTIVITY_CALLBACK)) return

                    val request = extractNetworkRequest(nri)
                    if (request != null && request.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) {
                        // App is specifically listening for a VPN network —
                        // suppress so it never learns one exists.
                        param.result = null
                        LsposedStats.record(uid, HookIds.Hook.LSPOSED_CONNECTIVITY_CALLBACK)
                        HookLog.i("VpnHide-CB: uid=$uid suppressed VPN-request dispatch")
                        return
                    }
                    (param.args.getOrNull(CALLBACK_BUNDLE_ARG_INDEX) as? Bundle)?.let {
                        if (sanitizeCallbackBundle(it)) {
                            LsposedStats.record(uid, HookIds.Hook.LSPOSED_CONNECTIVITY_CALLBACK)
                        }
                    }
                    currentCallbackUid.set(uid)
                }

                override fun afterHookedMethod(param: MethodHookParam) {
                    currentCallbackUid.remove()
                }
            }

        for (method in CALLBACK_DISPATCH_METHODS) {
            val hooked = XposedBridge.hookAllMethods(csClass, method, dispatchHook)
            if (hooked.isEmpty()) {
                HookLog.e("VpnHide: no $method on ${csClass.name}")
            } else {
                HookLog.i("VpnHide: hooked ConnectivityService.$method (${hooked.size})")
            }
        }
    }

    private fun installConnectivityServiceResultHooks(csClass: Class<*>) {
        for ((method, uidArgIndex) in CONNECTIVITY_RESULT_METHODS) {
            val hooked =
                XposedBridge.hookAllMethods(
                    csClass,
                    method,
                    object : XC_MethodHook() {
                        override fun afterHookedMethod(param: MethodHookParam) {
                            rememberConnectivityService(param.thisObject)
                            // getNetworkInfo(int networkType): an app that probes the
                            // legacy VPN type directly (getNetworkInfo(TYPE_VPN)) must
                            // get null — disguising the result as a *connected* WIFI
                            // NetworkInfo still answers "a VPN-type network exists and
                            // is connected", which isConnectedOrConnecting() reports as
                            // true. See issue #85 (Улыбка радуги).
                            if (method in LEGACY_TYPE_INFO_METHODS && suppressLegacyVpnTypeQuery(param)) return
                            val explicitUid = uidArgIndex?.let { param.args.getOrNull(it) as? Int }
                            sanitizeMethodResult(param, explicitUid)
                        }
                    },
                )
            if (hooked.isEmpty()) {
                HookLog.e("VpnHide: no ConnectivityService.$method result hook target on ${csClass.name}")
            } else {
                HookLog.i("VpnHide: hooked ConnectivityService.$method result (${hooked.size})")
            }
        }
    }

    private fun installConnectivityServiceNetworkHooks(csClass: Class<*>) {
        hookConnectivityNetworkMethod(csClass, "getActiveNetwork", ::sanitizeActiveNetworkResult)
        hookConnectivityNetworkMethod(csClass, "getAllNetworks", ::sanitizeAllNetworksResult)
        hookConnectivityNetworkMethod(csClass, "getNetworkForType", ::sanitizeNetworkForTypeResult)
    }

    private fun hookConnectivityNetworkMethod(
        csClass: Class<*>,
        method: String,
        sanitizer: (XC_MethodHook.MethodHookParam) -> Unit,
    ) {
        val hooked =
            XposedBridge.hookAllMethods(
                csClass,
                method,
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        rememberConnectivityService(param.thisObject)
                        if (bypassConnectivitySanitize.get() == true) return
                        if (!isTargetCallerOrUid(HookIds.Hook.LSPOSED_CONNECTIVITY_NETWORK)) return
                        sanitizer(param)
                    }
                },
            )
        if (hooked.isEmpty()) {
            HookLog.e("VpnHide: no ConnectivityService.$method network hook target on ${csClass.name}")
        } else {
            HookLog.i("VpnHide: hooked ConnectivityService.$method network result (${hooked.size})")
        }
    }

    private fun sanitizeActiveNetworkResult(param: XC_MethodHook.MethodHookParam) {
        val network = param.result as? Network ?: return
        val cs = param.thisObject ?: return
        if (!isVpnNetwork(cs, network)) return
        val replacement = findPhysicalNetwork(cs)
        if (replacement == null) {
            HookLog.i(
                "VpnHide: kept active VPN Network handle for uid=${effectiveCallerUid()}; " +
                    "no physical replacement found",
            )
            return
        }
        param.result = replacement
        LsposedStats.record(effectiveCallerUid(), HookIds.Hook.LSPOSED_CONNECTIVITY_NETWORK)
        HookLog.i("VpnHide: replaced active VPN Network handle for uid=${effectiveCallerUid()}")
    }

    private fun sanitizeAllNetworksResult(param: XC_MethodHook.MethodHookParam) {
        val networks = (param.result as? Array<*>)?.filterIsInstance<Network>() ?: return
        val cs = param.thisObject ?: return
        val filtered = networks.filterNot { isVpnNetwork(cs, it) }
        if (filtered.size == networks.size) return
        param.result = filtered.toTypedArray()
        LsposedStats.record(effectiveCallerUid(), HookIds.Hook.LSPOSED_CONNECTIVITY_NETWORK)
        HookLog.i(
            "VpnHide: filtered ${networks.size - filtered.size} VPN Network handle(s) " +
                "for uid=${effectiveCallerUid()}",
        )
    }

    private fun sanitizeNetworkForTypeResult(param: XC_MethodHook.MethodHookParam) {
        val type = param.args.getOrNull(0) as? Int ?: return
        if (type != ConnectivityManager.TYPE_VPN || param.result == null) return
        param.result = null
        LsposedStats.record(effectiveCallerUid(), HookIds.Hook.LSPOSED_CONNECTIVITY_NETWORK)
        HookLog.i("VpnHide: suppressed getNetworkForType(TYPE_VPN) for uid=${effectiveCallerUid()}")
    }

    // getNetworkInfo(int networkType) — null the result for a target caller that
    // probes TYPE_VPN directly. Returns true when the call was a TYPE_VPN query
    // (handled: either already null or nulled here), so the caller skips the
    // disguise-as-WIFI path that would otherwise leak a connected NetworkInfo.
    // Non-VPN types and non-target callers fall through to normal sanitizing.
    private fun suppressLegacyVpnTypeQuery(param: XC_MethodHook.MethodHookParam): Boolean {
        val type = param.args.getOrNull(0) as? Int ?: return false
        if (type != ConnectivityManager.TYPE_VPN) return false
        if (param.result == null) return true
        if (!isTargetCallerOrUid(HookIds.Hook.LSPOSED_CONNECTIVITY_RESULT)) return false
        param.result = null
        LsposedStats.record(effectiveCallerUid(), HookIds.Hook.LSPOSED_CONNECTIVITY_RESULT)
        HookLog.i("VpnHide: suppressed getNetworkInfo(TYPE_VPN) for uid=${effectiveCallerUid()}")
        return true
    }

    // The callback recipient UID lives on the NetworkRequestInfo arg under
    // different field names across AOSP versions (mAsUid is the UID the callback
    // is delivered as). Returns -1 if none found.
    private fun extractRecipientUid(nri: Any): Int {
        for (field in RECIPIENT_UID_FIELDS) {
            try {
                return XposedHelpers.getIntField(nri, field)
            } catch (_: Throwable) {
            }
        }
        return -1
    }

    // Find the NetworkRequest on the NRI by type — field name varies, and some
    // versions hold a list of requests (take the first). Flattened to a field
    // sequence over the class hierarchy to keep nesting shallow.
    private fun extractNetworkRequest(nri: Any): android.net.NetworkRequest? =
        generateSequence(nri.javaClass as Class<*>?) { it.superclass }
            .takeWhile { it != Any::class.java }
            .flatMap { it.declaredFields.asSequence() }
            .firstNotNullOfOrNull { field ->
                asNetworkRequest(
                    runCatching {
                        field.isAccessible = true
                        field.get(nri)
                    }.getOrNull(),
                )
            }

    private fun asNetworkRequest(value: Any?): android.net.NetworkRequest? =
        when (value) {
            is android.net.NetworkRequest -> value
            is List<*> -> value.firstOrNull { it is android.net.NetworkRequest } as? android.net.NetworkRequest
            else -> null
        }

    companion object {
        private const val SYSTEM_UID = 1000
        private const val CALLBACK_BUNDLE_ARG_INDEX = 2
        private val RECIPIENT_UID_FIELDS = listOf("mAsUid", "mUid", "uid")
        private val CALLBACK_DISPATCH_METHODS = listOf("callCallbackForRequest", "sendPendingIntentForRequest")

        // Result methods that take a legacy connectivity type as arg 0 and may be
        // queried with TYPE_VPN to probe for an active VPN (issue #85).
        private val LEGACY_TYPE_INFO_METHODS = setOf("getNetworkInfo", "getNetworkInfoForType")
        private val CONNECTIVITY_RESULT_METHODS =
            listOf(
                "getActiveLinkProperties" to null,
                "getLinkProperties" to null,
                "getLinkPropertiesForType" to null,
                "getRedactedLinkPropertiesForPackage" to 1,
                "getNetworkCapabilities" to null,
                "getDefaultNetworkCapabilitiesForUser" to null,
                "getRedactedNetworkCapabilitiesForPackage" to 1,
                "getActiveNetworkInfo" to null,
                "getActiveNetworkInfoForUid" to 0,
                "getNetworkInfo" to null,
                "getNetworkInfoForUid" to 1,
                "getAllNetworkInfo" to null,
            )
        private val FIELD_PROBES =
            listOf(
                FieldProbe(
                    "LinkProperties.mIfaceName",
                    LinkProperties::class.java,
                    "mIfaceName",
                ) { it == String::class.java },
                FieldProbe(
                    "LinkProperties.mRoutes",
                    LinkProperties::class.java,
                    "mRoutes",
                ) { MutableList::class.java.isAssignableFrom(it) },
                FieldProbe(
                    "LinkProperties.mStackedLinks",
                    LinkProperties::class.java,
                    "mStackedLinks",
                ) { MutableMap::class.java.isAssignableFrom(it) },
                FieldProbe(
                    "NetworkInfo.mNetworkType",
                    NetworkInfo::class.java,
                    "mNetworkType",
                ) { it == Integer.TYPE },
                FieldProbe(
                    "NetworkInfo.mState",
                    NetworkInfo::class.java,
                    "mState",
                ) { it == NetworkInfo.State::class.java },
                FieldProbe(
                    "NetworkInfo.mDetailedState",
                    NetworkInfo::class.java,
                    "mDetailedState",
                ) { it == NetworkInfo.DetailedState::class.java },
                FieldProbe(
                    "NetworkInfo.mIsAvailable",
                    NetworkInfo::class.java,
                    "mIsAvailable",
                ) { it == java.lang.Boolean.TYPE },
            )

        private val CTOR_PROBES =
            listOf(
                CtorProbe(
                    "LinkProperties.<init>(LinkProperties)",
                    LinkProperties::class.java,
                    arrayOf(LinkProperties::class.java),
                ),
                CtorProbe(
                    "NetworkInfo.<init>(int,int,String,String)",
                    NetworkInfo::class.java,
                    arrayOf(Integer.TYPE, Integer.TYPE, String::class.java, String::class.java),
                ),
            )

        // Per-hook critical-probe sets. A hook is skipped if any key in
        // its set is in the broken list. mRoutes / mStackedLinks are
        // intentionally NOT critical — graceful
        // degradation lives in the existing inner try/catch blocks.
        private val LP_CRITICAL_KEYS =
            setOf(
                "LinkProperties.mIfaceName",
                "LinkProperties.<init>(LinkProperties)",
            )
        private val NI_CRITICAL_KEYS =
            setOf(
                "NetworkInfo.mNetworkType",
                "NetworkInfo.mState",
                "NetworkInfo.mDetailedState",
                "NetworkInfo.mIsAvailable",
                "NetworkInfo.<init>(int,int,String,String)",
            )
    }
}
