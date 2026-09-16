package dev.okhsunrog.vpnhide.hook

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import android.net.RouteInfo
import android.os.Binder
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import de.robv.android.xposed.IXposedHookLoadPackage
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers
import de.robv.android.xposed.callbacks.XC_LoadPackage
import dev.okhsunrog.vpnhide.LsposedStats
import dev.okhsunrog.vpnhide.bit
import dev.okhsunrog.vpnhide.diagnostics.token
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
 * Single Xposed entry point for system_server hook wiring. Compatibility
 * probes and other self-contained collaborators live in adjacent Hook* files;
 * this class owns only stateful hook installation and callback coordination.
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

    // The cover network a VPN callback dispatch is being rewritten to, for the
    // duration of that dispatch. When set, the Network/NC/LP writeToParcel hooks
    // substitute the cover's own handle and facts instead of stripping the VPN's,
    // so the pushed handle and the pushed capabilities/link-properties describe
    // one and the same network (issue #70 coherence).
    private val callbackCover = ThreadLocal<Network>()

    @Volatile private var connectivityServiceInstance: Any? = null

    // ── ConnectivityService attach telemetry ──────────────────────────────
    // The CS hooks are the ones that leak on some devices while the framework
    // writeToParcel hooks work. Because attachment is deferred and multi-path
    // (A: system_server classloader now, B: live binder now, C: addService
    // later), logcat can't reliably show whether/how they attached — proven on
    // a working device where debug is on yet no install lines appear. So we
    // accumulate the outcome (resolved class, classloader chain, path, per-
    // method match counts, every attempt) and publish it to the LSPosed state
    // file, where it lands in hook_report.txt of any debug bundle regardless of
    // logcat or the debug-logging toggle.
    private val connectivityDiagnostics = ConnectivityAttachDiagnostics()

    // Background poller for path D (deferred getService retry). Created only if D
    // is actually needed (A/B didn't already attach) and torn down the moment the
    // hooks land or we give up — no idle thread lingering in system_server.
    @Volatile private var connectivityRetryThread: HandlerThread? = null

    @Volatile private var connectivityRetryHandler: Handler? = null

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
                installedMask = installedMask or HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY.bit
            }
            // ConnectivityService hooks attach asynchronously (the reliable path
            // only fires when "connectivity" registers, after this returns), so
            // their bits are NOT set here. ConnectivityAttachDiagnostics sets them
            // via LsposedStats once methods actually match — so the published mask
            // reflects real attachment and a device where they never attach shows
            // them under "missing owned hooks" instead of a false "8/8 installed".
            tryHook("ConnectivityService", installFailures) { installConnectivityServiceHook(lpparam.classLoader) }
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
            // Unchecked only in the element type: AOSP declares LinkProperties.mRoutes as
            // ArrayList<RouteInfo>, and the `as?` still checks List-ness at runtime, so a
            // ROM that reshaped the field falls out as null rather than crashing here.
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
            // Same shape as mRoutes above: AOSP declares mStackedLinks as
            // Hashtable<String, LinkProperties>; Map-ness is still checked at runtime.
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
        // NetworkCapabilities.getTransportInfo() is API 29+; VpnTransportInfo does
        // not exist on Android 9, so there is nothing to clear there.
        val hasVpnInfo =
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
                copy.transportInfo?.javaClass?.name == "android.net.VpnTransportInfo"

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

    private fun rawLinkProperties(
        cs: Any,
        network: Network,
    ): LinkProperties? =
        withClearedCallingIdentity {
            withConnectivitySanitizeBypassed {
                runCatching { XposedHelpers.callMethod(cs, "getLinkProperties", network) as? LinkProperties }.getOrNull()
            }
        }

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
    private val visibleNetworks =
        VisibleNetworkResolver(
            rawCapabilities = ::rawNetworkCapabilities,
            heuristicOrder = ::physicalNetworksByScore,
        )

    // The cover network for a uid, resolved the way AOSP resolves the network
    // behind its VPN (underlying → default → heuristic). Null when the platform
    // resolves none, in which case the caller must not invent one.
    private fun coverNetworkFor(
        cs: Any,
        uid: Int,
    ): Network? = visibleNetworks.coverFor(cs, uid)

    // The current callback dispatch's cover capabilities / link properties, read
    // from the live service. Null outside a VPN callback dispatch, or when the
    // service or the cover is unavailable — the caller then falls back to strip.
    private fun callbackCoverCapabilities(): NetworkCapabilities? {
        val cover = callbackCover.get() ?: return null
        val cs = connectivityServiceInstance ?: return null
        return rawNetworkCapabilities(cs, cover)
    }

    private fun callbackCoverLinkProperties(): LinkProperties? {
        val cover = callbackCover.get() ?: return null
        val cs = connectivityServiceInstance ?: return null
        return rawLinkProperties(cs, cover)
    }

    // The last-resort ordering the resolver falls back to on a ROM without the
    // per-uid AOSP methods: non-VPN networks with a physical transport, best first.
    private fun physicalNetworksByScore(cs: Any): List<Network> =
        rawAllNetworks(cs)
            .mapNotNull { network ->
                val caps = rawNetworkCapabilities(cs, network) ?: return@mapNotNull null
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) || !hasPhysicalTransport(caps)) {
                    null
                } else {
                    network to physicalNetworkScore(caps)
                }
            }.sortedByDescending { it.second }
            .map { it.first }

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
    //
    // A connected VPN NetworkInfo becomes the platform's *disconnected VPN* type,
    // not a connected Wi-Fi: the old Wi-Fi disguise answered "a Wi-Fi network is
    // connected", which contradicts the handle (now null, §nullVpnHandleResult)
    // and the legacy type answer (disconnected VPN, #337). This branch is a
    // backstop — the by-network paths null a VPN handle before it reaches here —
    // so it must not invent a connected network of any kind. Inactive VPN is left
    // exactly as the platform built it (#337).
    @Suppress("DEPRECATION")
    private fun sanitizedNetworkInfo(ni: NetworkInfo): NetworkInfo {
        val type = XposedHelpers.getIntField(ni, "mNetworkType")
        if (type != ConnectivityManager.TYPE_VPN || isInactiveVpnInfo(ni)) return ni

        val ctor =
            NetworkInfo::class.java.getDeclaredConstructor(
                Integer.TYPE,
                Integer.TYPE,
                String::class.java,
                String::class.java,
            )
        ctor.isAccessible = true
        val copy = ctor.newInstance(ConnectivityManager.TYPE_VPN, 0, "VPN", "") as NetworkInfo
        XposedHelpers.setObjectField(copy, "mState", NetworkInfo.State.DISCONNECTED)
        XposedHelpers.setObjectField(copy, "mDetailedState", NetworkInfo.DetailedState.DISCONNECTED)
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
        val installedHookMask: Long,
        val installFailures: List<String>,
    )

    private fun installSystemServerHooks(): HookInstallResult {
        val brokenFields = runHookReflectionSmokeCheck()
        if (brokenFields.isNotEmpty()) {
            HookLog.e("VpnHide: reflection smoke-check found broken keys: $brokenFields")
        }
        var installedHookMask = 0L
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
                installedHookMask = installedHookMask or HookIds.Hook.LSPOSED_LINK_PROPERTIES.bit
            }
        }

        // NC uses public NetworkCapabilities mutators now, so private AOSP
        // field drift must not disable this hook.
        if (tryHook("NC.writeToParcel", installFailures) { hookNCWriteToParcel() }) {
            installedHookMask = installedHookMask or HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES.bit
        }

        // NI: every field + ctor is critical — the hook body has no
        // inner try/catch around the per-field setIntField/setBooleanField
        // calls, so any rename would fail-open per call with logcat spam.
        if (anyBroken(NI_CRITICAL_KEYS)) {
            HookLog.e("VpnHide: NI.writeToParcel hook SKIPPED — critical reflection broken")
            installFailures += "NI.writeToParcel: skipped critical reflection broken: ${brokenFields.joinToString(",")}"
        } else {
            if (tryHook("NI.writeToParcel", installFailures) { hookNIWriteToParcel() }) {
                installedHookMask = installedHookMask or HookIds.Hook.LSPOSED_NETWORK_INFO.bit
            }
        }
        if (tryHook("Network.writeToParcel", installFailures) { hookNetworkWriteToParcel() }) {
            installedHookMask = installedHookMask or HookIds.Hook.LSPOSED_NETWORK.bit
        }

        tryHook("FileObserver", installFailures) { watchCanonicalConfigFile() }
        return HookInstallResult(brokenFields, installedHookMask, installFailures)
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
                        // In a VPN callback dispatch, hand the app the cover network's
                        // own capabilities so they match the cover handle it receives,
                        // rather than a transport-less strip of the VPN's.
                        val substitute = if (hasVpn) callbackCoverCapabilities() else null
                        val copy =
                            if (substitute != null) {
                                substitute
                            } else {
                                NetworkCapabilities(nc).also { if (!sanitizeNetworkCapabilities(it)) return }
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
                        LsposedStats.record(callerUid, HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES)
                        HookLog.i("VpnHide-NC: uid=$callerUid ${if (substitute != null) "SUBSTITUTED cover" else "STRIPPED VPN"}")
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
                        // In a callback dispatch, the cover was already chosen for the
                        // whole event; reuse it so the handle matches the NC/LP.
                        val replacement = callbackCover.get() ?: coverNetworkFor(cs, effectiveCallerUid()) ?: return
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
                        if (copy === ni) return

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
                        HookLog.i("VpnHide-NI: uid=$callerUid STRIPPED VPN (forced disconnected VPN)")
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
                    // Only act inside a VPN callback dispatch, and only to substitute the
                    // cover's own link properties for the VPN's. The synchronous LP paths
                    // are already handled by the result hooks (a VPN handle is nulled, a
                    // physical network passes through), so name-stripping here would only
                    // blank a real carrier IMS ipsec*/xfrm* interface that is not a VPN.
                    val cover = callbackCover.get() ?: return
                    val isVpnLp = ifname != null && isVpnInterfaceName(ifname)
                    if (!isVpnLp) return
                    try {
                        val substitute = callbackCoverLinkProperties() ?: return
                        val parcel = param.args[0] as android.os.Parcel
                        val flags = param.args[1] as Int
                        writingCopy.set(true)
                        try {
                            substitute.writeToParcel(parcel, flags)
                        } finally {
                            writingCopy.set(false)
                        }
                        param.result = null
                        LsposedStats.record(callerUid, HookIds.Hook.LSPOSED_LINK_PROPERTIES)
                        HookLog.i("VpnHide-LP: uid=$callerUid SUBSTITUTED cover=$cover (ifname was $ifname)")
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
        connectivityDiagnostics.record(
            "install sdk=${Build.VERSION.SDK_INT} bootLoader=${connectivityDiagnostics.describeLoader(bootClassLoader)}",
        )
        // We deliberately do NOT attach via the system_server classloader at
        // install time (the old "path A"). Even where the class resolves (≤ A12),
        // hooking it here — before ConnectivityService is constructed — binds
        // method entries that ART then replaces when the class is initialised and
        // compiled, so the hooks silently never fire (seen on Redmi Note 8 Pro /
        // MediaTek A11: every method matched, mask full, yet all checks leaked and
        // none of the connectivity counters moved). Worse, that early attach grabbed
        // the once-only latch and blocked the late paths that DO stick. So we only
        // ever attach from the live service binder, once it exists: B (already up),
        // C (addService registration), D (deferred poll).
        val serviceManager = XposedHelpers.findClass("android.os.ServiceManager", bootClassLoader)
        // Path B — the service may already be registered; its binder carries the
        // real classloader (the Connectivity APEX loader on A13+).
        val existing = XposedHelpers.callStaticMethod(serviceManager, "getService", "connectivity") as? android.os.IBinder
        connectivityDiagnostics.record("B:getService=${existing?.javaClass?.simpleName ?: "null"}")
        existing?.let { hookConnectivityFromBinder(it, "B") }
        // Path C — catch the registration so we get the binder's classloader even
        // when it isn't up yet at install time.
        XposedBridge.hookAllMethods(
            serviceManager,
            "addService",
            object : XC_MethodHook() {
                override fun beforeHookedMethod(param: MethodHookParam) {
                    if (param.args.getOrNull(0) != "connectivity") return
                    connectivityDiagnostics.record("C:addService(connectivity) seen")
                    (param.args.getOrNull(1) as? android.os.IBinder)?.let { hookConnectivityFromBinder(it, "C") }
                }
            },
        )
        // Path D — deferred retrying getService(): the robust fallback. On A13+
        // OEM ROMs (MediaTek/MIUI) the service isn't published through the hooked
        // ServiceManager.addService, so path C never fires (proven on POCO A13);
        // but connectivity is always resolvable by name once it's up, so we
        // re-query on a short timer until the live binder appears and attach from
        // its (APEX) classloader — independent of the registration mechanism.
        startConnectivityRetry(serviceManager)
    }

    private fun startConnectivityRetry(serviceManager: Class<*>) {
        if (connectivityHooked.get()) return // A/B already got it — no poller needed
        val thread = HandlerThread("vpnhide-cs-attach").apply { start() }
        connectivityRetryThread = thread
        connectivityRetryHandler = Handler(thread.looper)
        scheduleConnectivityRetry(serviceManager, 1)
    }

    private fun stopConnectivityRetry() {
        connectivityRetryThread?.quitSafely()
        connectivityRetryThread = null
        connectivityRetryHandler = null
    }

    private fun scheduleConnectivityRetry(
        serviceManager: Class<*>,
        attempt: Int,
    ) {
        val handler = connectivityRetryHandler ?: return
        if (connectivityHooked.get()) {
            stopConnectivityRetry()
            return
        }
        if (attempt > CS_RETRY_MAX_ATTEMPTS) {
            connectivityDiagnostics.record("D:gaveUp after ${attempt - 1} tries")
            stopConnectivityRetry()
            return
        }
        handler.postDelayed({
            // Path C (or A/B) may have attached while we waited — done, tear down.
            if (connectivityHooked.get()) {
                stopConnectivityRetry()
                return@postDelayed
            }
            val binder =
                try {
                    XposedHelpers.callStaticMethod(serviceManager, "getService", "connectivity") as? android.os.IBinder
                } catch (_: Throwable) {
                    null
                }
            if (binder == null) {
                // Not registered yet — keep waiting.
                scheduleConnectivityRetry(serviceManager, attempt + 1)
                return@postDelayed
            }
            connectivityDiagnostics.record("D:getService(try=$attempt)=${binder.javaClass.simpleName}")
            hookConnectivityFromBinder(binder, "D")
            // The name now resolves; retrying the same binder won't change the
            // outcome, so stop here whether or not the attach stuck.
            if (!connectivityHooked.get()) connectivityDiagnostics.record("D:attachFailed on resolved binder")
            stopConnectivityRetry()
        }, CS_RETRY_DELAY_MS)
    }

    private fun hookConnectivityFromBinder(
        binder: android.os.IBinder,
        path: String,
    ) {
        // Hook the binder's OWN runtime class, never a name-resolved one. On some
        // ROMs (MediaTek A11) the live ConnectivityService is defined by a child
        // classloader, and findClass(name, thatLoader) follows delegation to a
        // *parent* copy of the class — a different Class object with the same name.
        // Hooking that copy attaches cleanly (every method matches) yet never
        // fires, because the live binder dispatches to the child copy (attached,
        // all counts=1, but no connectivity counter ever moves). binder.javaClass
        // IS the live class, so hooking it always intercepts the real calls.
        val csClass = binder.javaClass
        val loader = csClass.classLoader
        // Permanent guard: does name-resolution return this same live class?
        // nameResolvesSame=false is the delegation trap above — a one-line tell in
        // the report if another ROM ever loads ConnectivityService oddly.
        val nameResolvesSame =
            loader?.let { runCatching { findConnectivityServiceClass(it) === csClass }.getOrNull() }
        connectivityDiagnostics.record(
            "$path:binderClass=${csClass.name} loader=${connectivityDiagnostics.describeLoader(loader)} " +
                "nameResolvesSame=$nameResolvesSame",
        )
        hookConnectivityServiceIfPossible(csClass, path)
    }

    private fun hookConnectivityServiceIfPossible(
        csClass: Class<*>,
        path: String,
    ) {
        if (!connectivityHooked.compareAndSet(false, true)) {
            connectivityDiagnostics.record("$path:skipped(already-hooked)")
            return
        }
        try {
            hookConnectivityService(csClass, path)
        } catch (t: Throwable) {
            connectivityHooked.set(false)
            connectivityDiagnostics.record("$path:failed(${t.javaClass.simpleName}:${t.message}) class=${csClass.name}")
            HookLog.i("VpnHide: ConnectivityService hook failed on ${csClass.name}: ${t.message}")
        }
    }

    // Kept only for the hookConnectivityFromBinder diagnostic that proves the
    // name-resolution-vs-live-class mismatch; the attach itself uses the binder's
    // own class and never this.
    private fun findConnectivityServiceClass(classLoader: ClassLoader): Class<*> =
        try {
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
    private fun hookConnectivityService(
        csClass: Class<*>,
        path: String,
    ) {
        val ctorCount =
            try {
                XposedBridge
                    .hookAllConstructors(
                        csClass,
                        object : XC_MethodHook() {
                            override fun afterHookedMethod(param: MethodHookParam) {
                                rememberConnectivityService(param.thisObject)
                            }
                        },
                    ).size
            } catch (t: Throwable) {
                HookLog.e("VpnHide: ConnectivityService constructors hook failed: ${t.message}")
                0
            }
        val resultCounts = installConnectivityServiceResultHooks(csClass)
        val networkCounts = installConnectivityServiceNetworkHooks(csClass)

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
                        suppressDispatch(param, uid, "VPN-request")
                        return
                    }
                    // A PendingIntent is parcelled asynchronously by the broadcast
                    // queue, outside this dispatch — the writeToParcel hooks never see
                    // it — so its VPN network must be swapped for the cover here, on
                    // the argument itself, rather than via the callback-cover context.
                    if (param.method.name == PENDING_INTENT_DISPATCH_METHOD) {
                        preparePendingIntentDispatch(param, uid, request)
                    } else {
                        prepareDispatch(param, uid, request)
                    }
                }

                override fun afterHookedMethod(param: MethodHookParam) {
                    currentCallbackUid.remove()
                    callbackCover.remove()
                }
            }

        val callbackCounts = LinkedHashMap<String, Int>()
        for (method in CALLBACK_DISPATCH_METHODS) {
            val hooked = XposedBridge.hookAllMethods(csClass, method, dispatchHook)
            callbackCounts[method] = hooked.size
            if (hooked.isEmpty()) {
                HookLog.e("VpnHide: no $method on ${csClass.name}")
            } else {
                HookLog.i("VpnHide: hooked ConnectivityService.$method (${hooked.size})")
            }
        }

        connectivityDiagnostics.report(
            path,
            csClass,
            csClass.classLoader,
            ctorCount,
            resultCounts,
            networkCounts,
            callbackCounts,
        )
    }

    private fun installConnectivityServiceResultHooks(csClass: Class<*>): Map<String, Int> {
        val counts = LinkedHashMap<String, Int>()
        for ((method, uidArgIndex) in CONNECTIVITY_RESULT_METHODS) {
            val hooked =
                XposedBridge.hookAllMethods(
                    csClass,
                    method,
                    object : XC_MethodHook() {
                        override fun afterHookedMethod(param: MethodHookParam) {
                            rememberConnectivityService(param.thisObject)
                            if (param.hasThrowable() || bypassConnectivitySanitize.get() == true) return
                            // A by-network query (getNetworkCapabilities/getLinkProperties/
                            // getNetworkInfoForUid) is fully handled by handle identity: a VPN
                            // handle is nulled (unknown-netId semantics), any other network
                            // passes through untouched so a carrier IMS ipsec* interface is
                            // not name-stripped.
                            if (handleByNetworkResult(method, param)) return
                            // Legacy type queries must retain the platform's disconnected
                            // VPN entry. A null or a connected WIFI replacement is observable.
                            if (sanitizeLegacyNetworkInfoResult(method, param)) return
                            val explicitUid = uidArgIndex?.let { param.args.getOrNull(it) as? Int }
                            sanitizeMethodResult(param, explicitUid)
                        }
                    },
                )
            // Only record methods that exist on this build — a 0 for a method the
            // AOSP version simply doesn't have is noise; a 0 for one it *does*
            // have is the signal. Keep every entry; the reader compares to a
            // known-good device.
            counts[method] = hooked.size
            if (hooked.isEmpty()) {
                val minApi = CONNECTIVITY_RESULT_METHOD_MIN_API[method]
                if (minApi != null && Build.VERSION.SDK_INT < minApi) {
                    // The method (and the detection vector it covers) doesn't exist
                    // on this Android version — absence is expected, not an error.
                    HookLog.i("VpnHide: ConnectivityService.$method absent on API ${Build.VERSION.SDK_INT} (added in API $minApi)")
                } else {
                    HookLog.e("VpnHide: no ConnectivityService.$method result hook target on ${csClass.name}")
                }
            } else {
                HookLog.i("VpnHide: hooked ConnectivityService.$method result (${hooked.size})")
            }
        }
        return counts
    }

    private fun installConnectivityServiceNetworkHooks(csClass: Class<*>): Map<String, Int> =
        linkedMapOf(
            "getActiveNetwork" to hookConnectivityNetworkMethod(csClass, "getActiveNetwork", ::sanitizeActiveNetworkResult),
            "getAllNetworks" to hookConnectivityNetworkMethod(csClass, "getAllNetworks", ::sanitizeAllNetworksResult),
            "getNetworkForType" to hookConnectivityNetworkMethod(csClass, "getNetworkForType", ::sanitizeNetworkForTypeResult),
        )

    private fun hookConnectivityNetworkMethod(
        csClass: Class<*>,
        method: String,
        sanitizer: (XC_MethodHook.MethodHookParam) -> Unit,
    ): Int {
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
        return hooked.size
    }

    private fun sanitizeActiveNetworkResult(param: XC_MethodHook.MethodHookParam) {
        val network = param.result as? Network ?: return
        val cs = param.thisObject ?: return
        if (!isVpnNetwork(cs, network)) return
        val replacement = coverNetworkFor(cs, effectiveCallerUid())
        if (replacement == null) {
            HookLog.i(
                "VpnHide: kept active VPN Network handle for uid=${effectiveCallerUid()}; " +
                    "no cover network resolved",
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

    // A ConnectivityService result method that takes the queried Network as its
    // first argument (getNetworkCapabilities, getLinkProperties, getNetworkInfo-
    // ForUid) is fully handled here for a target, so the caller skips the generic
    // name-based sanitizer:
    //  - a VPN Network is answered null — the AOSP behaviour for a netId that no
    //    longer exists, so a retained or scanned handle describes no network;
    //  - any other network passes through untouched. A carrier IMS network runs
    //    over an `ipsec*`/`xfrm*` interface yet is marked NOT_VPN; name-stripping
    //    its link properties blanked a real physical network's interface. Only a
    //    network that actually carries TRANSPORT_VPN is hidden, and that one is
    //    nulled above, so a by-network query never strips by interface name.
    // Returns true when the method is a by-network one and the caller is a target.
    private fun handleByNetworkResult(
        method: String,
        param: XC_MethodHook.MethodHookParam,
    ): Boolean {
        if (method !in BY_NETWORK_RESULT_METHODS) return false
        // The first argument identifies the network: a Network for the public
        // overloads, or a NetworkAgentInfo for the private getLinkProperties(nai)
        // that the public one delegates to (its `network` field). Both must be
        // handled, or the private overload's result is name-stripped before the
        // public overload returns it — which blanked carrier IMS ipsec* links.
        val arg0 = param.args.getOrNull(0) ?: return false
        val network =
            arg0 as? Network
                ?: runCatching { XposedHelpers.getObjectField(arg0, "network") as? Network }.getOrNull()
                ?: return false
        // getNetworkInfoForUid(network, uid, ignoreBlocked): the uid the answer is
        // for is the second arg; the other two are plain caller queries.
        val explicitUid = if (method == "getNetworkInfoForUid") param.args.getOrNull(1) as? Int else null
        if (!isTargetCallerOrUid(HookIds.Hook.LSPOSED_CONNECTIVITY_RESULT, explicitUid)) return false
        val cs = param.thisObject ?: return false
        if (param.result != null && isVpnNetwork(cs, network)) {
            param.result = null
            LsposedStats.record(explicitUid ?: effectiveCallerUid(), HookIds.Hook.LSPOSED_CONNECTIVITY_RESULT)
            HookLog.i("VpnHide: nulled $method for a VPN handle, uid=${explicitUid ?: effectiveCallerUid()}")
        }
        return true
    }

    private fun sanitizeLegacyNetworkInfoResult(
        method: String,
        param: XC_MethodHook.MethodHookParam,
    ): Boolean {
        val requestedType = if (method in LEGACY_TYPE_INFO_METHODS) param.args.getOrNull(0) as? Int else null
        if (requestedType != ConnectivityManager.TYPE_VPN && method != "getAllNetworkInfo") return false
        // Handle VPN type queries, including null and non-target replies,
        // so none accidentally falls through to the generic VPN-to-WIFI conversion.
        if (!isTargetCallerOrUid(HookIds.Hook.LSPOSED_CONNECTIVITY_RESULT)) return true
        val uid = effectiveCallerUid()
        try {
            fun normalize(info: NetworkInfo?): NetworkInfo? =
                normalizeLegacyVpnInfo(info, requestedType, NetworkInfo::getType, ::isInactiveVpnInfo) {
                    withConnectivitySanitizeBypassed { disconnectedVpnInfo(param.thisObject, uid) }
                }
            val original = param.result
            val replacement =
                when (original) {
                    is NetworkInfo -> {
                        normalize(original)
                    }

                    is Array<*> -> {
                        if (original.javaClass.componentType != NetworkInfo::class.java) return true
                        val copy = original.map { normalize(it as? NetworkInfo) }.toTypedArray()
                        if (copy.indices.all { copy[it] === original[it] }) original else copy
                    }

                    else -> {
                        original
                    }
                }
            if (replacement !== original) {
                param.result = replacement
                LsposedStats.record(uid, HookIds.Hook.LSPOSED_CONNECTIVITY_RESULT)
            }
        } catch (t: Throwable) {
            // Unknown OEM policy: retain the original result/exception rather than
            // inventing a disconnected state that ignores that platform's restrictions.
            HookLog.e("VpnHide: legacy NetworkInfo normalize error: ${t.message}")
        }
        return true
    }

    // Decide what to do with a non-VPN-requested dispatch to a target: a VPN
    // network is either suppressed (a passive listen must not reveal it exists) or
    // rewritten to the cover (a default/request callback must still deliver *a*
    // network — the cover — so the app is not left thinking it is offline). A
    // physical network, or a dispatch with no resolvable network, passes through
    // with only the recipient uid stashed for the parcel hooks.
    private fun prepareDispatch(
        param: XC_MethodHook.MethodHookParam,
        uid: Int,
        request: android.net.NetworkRequest?,
    ) {
        val cs = connectivityServiceInstance
        val network = dispatchedNetwork(param)
        if (cs == null || network == null || !isVpnNetwork(cs, network)) {
            currentCallbackUid.set(uid)
            return
        }
        if (requestIsListen(request)) {
            suppressDispatch(param, uid, "VPN-listen")
            return
        }
        val cover = coverNetworkFor(cs, uid)
        if (cover == null) {
            suppressDispatch(param, uid, "VPN-default (no cover)")
            return
        }
        callbackCover.set(cover)
        currentCallbackUid.set(uid)
    }

    // A PendingIntent dispatch delivers EXTRA_NETWORK from a NetworkAgentInfo
    // argument. When that network is the VPN, swap the argument for the cover's
    // NetworkAgentInfo before the method builds and sends the Intent (a passive
    // listen is suppressed instead). The Intent is then parcelled — asynchronously
    // — already carrying the cover, so no in-context parcel hook is needed.
    private fun preparePendingIntentDispatch(
        param: XC_MethodHook.MethodHookParam,
        uid: Int,
        request: android.net.NetworkRequest?,
    ) {
        val cs = connectivityServiceInstance ?: return
        val naiIndex =
            param.args.indexOfFirst {
                runCatching { XposedHelpers.getObjectField(it, "network") as? Network }.getOrNull() != null
            }
        if (naiIndex < 0) return
        val network = XposedHelpers.getObjectField(param.args[naiIndex], "network") as? Network ?: return
        if (!isVpnNetwork(cs, network)) return
        if (requestIsListen(request)) {
            suppressDispatch(param, uid, "VPN-listen PendingIntent")
            return
        }
        val cover = coverNetworkFor(cs, uid)
        val coverNai = cover?.let { runCatching { XposedHelpers.callMethod(cs, "getNetworkAgentInfoForNetwork", it) }.getOrNull() }
        if (coverNai == null) {
            suppressDispatch(param, uid, "VPN PendingIntent (no cover)")
            return
        }
        param.args[naiIndex] = coverNai
        LsposedStats.record(uid, HookIds.Hook.LSPOSED_CONNECTIVITY_CALLBACK)
        HookLog.i("VpnHide-CB: uid=$uid rewrote PendingIntent network to cover")
    }

    private fun suppressDispatch(
        param: XC_MethodHook.MethodHookParam,
        uid: Int,
        reason: String,
    ) {
        param.result = null
        LsposedStats.record(uid, HookIds.Hook.LSPOSED_CONNECTIVITY_CALLBACK)
        HookLog.i("VpnHide-CB: uid=$uid suppressed $reason dispatch")
    }

    // The network a callback dispatch is about: a NetworkAgentInfo argument exposes
    // it in its `network` field (the outer/legacy overloads), else the callback
    // Bundle carries it (the inner overload). Null for a lost/unavailable dispatch
    // that names no live network.
    private fun dispatchedNetwork(param: XC_MethodHook.MethodHookParam): Network? {
        param.args.forEach { arg ->
            if (arg is Network) return arg
            val fromNai = runCatching { XposedHelpers.getObjectField(arg, "network") as? Network }.getOrNull()
            if (fromNai != null) return fromNai
        }
        val bundle = param.args.getOrNull(CALLBACK_BUNDLE_ARG_INDEX) as? Bundle
        @Suppress("DEPRECATION")
        return bundle?.getParcelable(Network::class.java.simpleName)
    }

    // A passive LISTEN (or LISTEN_FOR_BEST) enumerates networks; a VPN match is
    // suppressed. Any other request type (TRACK_DEFAULT, REQUEST) wants a network
    // delivered, so its VPN match is rewritten to the cover instead. NetworkRequest
    // exposes its type publicly; an unreadable type is treated as non-listen so
    // connectivity is never silently withheld.
    private fun requestIsListen(request: android.net.NetworkRequest?): Boolean {
        val type = runCatching { XposedHelpers.getObjectField(request, "type")?.toString() }.getOrNull()
        return type == "LISTEN" || type == "LISTEN_FOR_BEST"
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

        // Path D poll cadence: connectivity registers within a few seconds of
        // boot; retry getService("connectivity") every 500ms for up to ~90s
        // (180 tries), stopping — and tearing down the poller thread — as soon as
        // the hooks attach.
        private const val CS_RETRY_DELAY_MS = 500L
        private const val CS_RETRY_MAX_ATTEMPTS = 180
        private val RECIPIENT_UID_FIELDS = listOf("mAsUid", "mUid", "uid")
        private const val PENDING_INTENT_DISPATCH_METHOD = "sendPendingIntentForRequest"
        private val CALLBACK_DISPATCH_METHODS = listOf("callCallbackForRequest", PENDING_INTENT_DISPATCH_METHOD)

        // Result methods that take a legacy connectivity type as arg 0 and may be
        // queried with TYPE_VPN to probe for an active VPN (issue #85).
        private val LEGACY_TYPE_INFO_METHODS = setOf("getNetworkInfo", "getNetworkInfoForType")

        // Result methods whose first argument is the queried Network — a VPN one
        // is answered null for a target (nullVpnHandleResult). getNetworkInfo(int)
        // is a legacy *type* query, not a by-Network one, and stays out of this set.
        private val BY_NETWORK_RESULT_METHODS =
            setOf("getNetworkCapabilities", "getLinkProperties", "getNetworkInfoForUid")
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

        // Result methods that AOSP only added in a later API. Below that level the
        // method (and the detection vector it covers) simply doesn't exist, so a
        // missing hook target is expected — log it at INFO, not ERROR.
        private val CONNECTIVITY_RESULT_METHOD_MIN_API =
            mapOf(
                // getRedacted*ForPackage arrived with the per-package redaction API.
                "getRedactedLinkPropertiesForPackage" to Build.VERSION_CODES.S,
                "getRedactedNetworkCapabilitiesForPackage" to Build.VERSION_CODES.S,
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
