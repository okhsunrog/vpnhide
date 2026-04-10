package dev.okhsunrog.vpnhide

import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import de.robv.android.xposed.IXposedHookLoadPackage
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers
import de.robv.android.xposed.callbacks.XC_LoadPackage
import java.io.File
import java.io.FileInputStream
import java.io.FileReader
import java.lang.reflect.Method
import java.net.InetAddress
import java.net.NetworkInterface
import java.util.Collections

/**
 * VpnHide — hide VPN presence from apps that do client-side VPN detection.
 *
 * Covers the detection paths a Java/Kotlin Android app can use to discover
 * an active VPN:
 *
 *   1. NetworkCapabilities — hasTransport(TRANSPORT_VPN), hasCapability(NOT_VPN),
 *      getTransportTypes(), getTransportInfo() (hides VpnTransportInfo on API 31+).
 *   2. NetworkInfo (legacy)  — getType() / getTypeName() returning TYPE_VPN / "VPN".
 *   3. ConnectivityManager   — getAllNetworks / getActiveNetwork / getActiveNetworkInfo /
 *      getAllNetworkInfo / getNetworkInfo(int) / getNetworkInfo(Network).
 *   4. LinkProperties        — getInterfaceName (returning "tun0" etc.), getRoutes (VPN routes),
 *      getDnsServers (empty list for VPN LPs), getHttpProxy (null for VPN LPs).
 *   5. NetworkInterface      — getNetworkInterfaces, getByName, getByIndex, getByInetAddress,
 *      hiding any interface whose name looks like a VPN tunnel (tun, ppp, tap, wg, ipsec,
 *      xfrm, utun, l2tp).
 *   6. /proc/net entries     — FileInputStream / FileReader constructors redirected to
 *      /dev/null for sensitive paths, so reading them yields empty content.
 *   7. System.getProperty    — returns null (or the caller's default) for JVM-level
 *      proxy keys (http.proxyHost, socksProxyHost, …) that both RKNHardering and
 *      YourVPNDead use as VPN signals.
 *
 * It does NOT cover:
 *   - Native getifaddrs() / direct ioctl calls from C/C++. Apps that use JNI for
 *     network enumeration bypass all Java hooks. A Zygisk native module would be
 *     needed for that.
 *   - Server-side detection (DNS poisoning, IP range checks, latency fingerprinting).
 *     These are network-layer and unfixable client-side.
 */
class HookEntry : IXposedHookLoadPackage {

    // Saved references to original Method objects so we can call them from inside
    // filters without hitting our own hooks. Framework classes live in the boot
    // classloader, so these Method references are the same across all hooked
    // packages — safe to keep on the singleton HookEntry instance.
    @Volatile private var origHasTransport: Method? = null
    @Volatile private var origGetInterfaceName: Method? = null

    override fun handleLoadPackage(lpparam: XC_LoadPackage.LoadPackageParam) {
        // Never hook ourselves
        if (lpparam.packageName == BuildConfig.APPLICATION_ID) return

        // Scope is controlled entirely by LSPosed/Vector's per-app scope setting.
        // We hook every process the framework loads us into. Pick target apps in
        // Vector manager → Modules → VPN Hide → scope.
        XposedBridge.log("VpnHide: installing hooks for ${lpparam.packageName}")
        installHooks(lpparam.classLoader)
    }

    private fun installHooks(cl: ClassLoader) {
        tryHook("NetworkCapabilities")  { hookNetworkCapabilities(cl) }
        tryHook("NetworkInfo")          { hookNetworkInfo(cl) }
        tryHook("ConnectivityManager")  { hookConnectivityManager(cl) }
        tryHook("LinkProperties")       { hookLinkProperties(cl) }
        tryHook("NetworkInterface")     { hookNetworkInterface() }
        tryHook("ProcNetFiles")         { hookProcNetFiles() }
        tryHook("SystemProperties")     { hookSystemProperties() }
    }

    private inline fun tryHook(name: String, block: () -> Unit) {
        try {
            block()
        } catch (t: Throwable) {
            XposedBridge.log("VpnHide: $name hook failed: ${t::class.java.simpleName}: ${t.message}")
        }
    }

    // ------------------------------------------------------------------
    //  NetworkCapabilities
    // ------------------------------------------------------------------

    private fun hookNetworkCapabilities(cl: ClassLoader) {
        val ncClass = XposedHelpers.findClass("android.net.NetworkCapabilities", cl)

        // hasTransport(int): always return false when asked about TRANSPORT_VPN.
        val hasTransport = XposedHelpers.findMethodExact(
            ncClass, "hasTransport", java.lang.Integer.TYPE
        )
        origHasTransport = hasTransport
        XposedBridge.hookMethod(hasTransport, object : XC_MethodHook() {
            override fun beforeHookedMethod(param: MethodHookParam) {
                if ((param.args[0] as Int) == TRANSPORT_VPN) {
                    param.result = false
                }
            }
        })

        // hasCapability(NET_CAPABILITY_NOT_VPN): always return true.
        XposedHelpers.findAndHookMethod(
            ncClass, "hasCapability", java.lang.Integer.TYPE,
            object : XC_MethodHook() {
                override fun beforeHookedMethod(param: MethodHookParam) {
                    if ((param.args[0] as Int) == NET_CAPABILITY_NOT_VPN) {
                        param.result = true
                    }
                }
            }
        )

        // getTransportTypes(): strip TRANSPORT_VPN from the returned int[].
        runCatching {
            XposedHelpers.findAndHookMethod(
                ncClass, "getTransportTypes",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val r = param.result as? IntArray ?: return
                        if (r.contains(TRANSPORT_VPN)) {
                            param.result = r.filter { it != TRANSPORT_VPN }.toIntArray()
                        }
                    }
                }
            )
        }

        // getTransportInfo() (API 31+): if the real TransportInfo is a
        // VpnTransportInfo, return null. Apps that know about API 31+ use
        // this as a direct "am I on a VPN?" probe that bypasses the
        // hasTransport hook entirely. We detect VpnTransportInfo by class
        // name via reflection so this file keeps compiling against older
        // compile SDKs.
        runCatching {
            XposedHelpers.findAndHookMethod(
                ncClass, "getTransportInfo",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val ti = param.result ?: return
                        if (ti.javaClass.name == "android.net.VpnTransportInfo") {
                            param.result = null
                        }
                    }
                }
            )
        }

        // toString(): NetworkCapabilities.toString() iterates the internal
        // mTransportTypes bitmask DIRECTLY, not via getTransportTypes(),
        // so our hooks above don't affect it. Apps can therefore call
        // `cm.getNetworkCapabilities(net).toString()` and grep for "VPN"
        // — the official Russian Ministry of Digital Development VPN
        // detection methodology lists exactly this output (e.g.
        // `Transports: WIFI|VPN`, `VpnTransportInfo{...}`) as a direct
        // sign. Strip the offending tokens from the returned string.
        runCatching {
            XposedHelpers.findAndHookMethod(
                ncClass, "toString",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val s = param.result as? String ?: return
                        param.result = sanitizeNetworkCapabilitiesString(s)
                    }
                }
            )
        }
    }

    /**
     * Strip every textual mention of a VPN transport from the formatted
     * NetworkCapabilities string. Handles three places where VPN can
     * appear in stock AOSP output:
     *
     *   1. `Transports: WIFI|VPN` — rewrite the pipe-separated list,
     *      removing any `VPN` element. Falls back to `WIFI` if VPN was
     *      the only entry, since we don't want to leave a dangling
     *      `Transports: ` line.
     *   2. `VpnTransportInfo{type=1, sessionId=..., ...}` — replace the
     *      whole object dump with `null` so it looks like
     *      `getTransportInfo()` returned null (which is consistent with
     *      our getTransportInfo hook).
     *   3. Any stray `IS_VPN` flag — defensive; some custom ROMs add
     *      it to the Capabilities list. Drop it from the &-joined list.
     */
    private fun sanitizeNetworkCapabilitiesString(s: String): String {
        var out = s

        // 1. Transports: <pipe-list>
        // Match the segment from "Transports: " up to the next whitespace.
        val transportsRe = Regex("""Transports:\s*([^\s\]]+)""")
        out = transportsRe.replace(out) { mr ->
            val list = mr.groupValues[1]
                .split('|')
                .filter { it.isNotEmpty() && it != "VPN" }
            val rebuilt = if (list.isEmpty()) "WIFI" else list.joinToString("|")
            "Transports: $rebuilt"
        }

        // 2. VpnTransportInfo{...}
        out = Regex("""VpnTransportInfo\{[^}]*}""").replace(out, "null")

        // 3. IS_VPN flag in any &-separated list (Capabilities, Policies).
        out = out
            .replace("&IS_VPN&", "&")
            .replace("IS_VPN&", "")
            .replace("&IS_VPN", "")
            .replace("IS_VPN", "")

        return out
    }

    /** Calls the *real* hasTransport, bypassing our own hook, to detect a VPN network. */
    private fun isVpnNetworkRaw(nc: NetworkCapabilities): Boolean {
        val m = origHasTransport ?: return false
        return try {
            XposedBridge.invokeOriginalMethod(m, nc, arrayOf(TRANSPORT_VPN)) as? Boolean ?: false
        } catch (_: Throwable) {
            false
        }
    }

    // ------------------------------------------------------------------
    //  NetworkInfo (legacy but still used by many Russian banking / gov apps)
    // ------------------------------------------------------------------

    private fun hookNetworkInfo(cl: ClassLoader) {
        val niClass = XposedHelpers.findClass("android.net.NetworkInfo", cl)

        // getType() == TYPE_VPN (17) → return TYPE_WIFI (1).
        XposedHelpers.findAndHookMethod(
            niClass, "getType",
            object : XC_MethodHook() {
                override fun afterHookedMethod(param: MethodHookParam) {
                    if (param.result == TYPE_VPN) param.result = TYPE_WIFI
                }
            }
        )

        // getTypeName() == "VPN" → return "WIFI".
        XposedHelpers.findAndHookMethod(
            niClass, "getTypeName",
            object : XC_MethodHook() {
                override fun afterHookedMethod(param: MethodHookParam) {
                    if (param.result == "VPN") param.result = "WIFI"
                }
            }
        )

        // getSubtypeName() — some implementations return "VPN" here.
        runCatching {
            XposedHelpers.findAndHookMethod(
                niClass, "getSubtypeName",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val s = param.result as? String ?: return
                        if (s.contains("VPN", ignoreCase = true)) param.result = ""
                    }
                }
            )
        }
    }

    // ------------------------------------------------------------------
    //  ConnectivityManager
    // ------------------------------------------------------------------

    private fun hookConnectivityManager(cl: ClassLoader) {
        val cmClass = XposedHelpers.findClass("android.net.ConnectivityManager", cl)

        // getAllNetworks(): remove VPN networks from the array.
        runCatching {
            XposedHelpers.findAndHookMethod(
                cmClass, "getAllNetworks",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val networks = param.result as? Array<*> ?: return
                        val cm = param.thisObject as ConnectivityManager
                        val filtered = networks.filterIsInstance<Network>().filter { n ->
                            val nc = cm.getNetworkCapabilities(n) ?: return@filter true
                            !isVpnNetworkRaw(nc)
                        }.toTypedArray()
                        param.result = filtered
                    }
                }
            )
        }

        // getActiveNetwork(): if it's a VPN, substitute the first non-VPN network.
        runCatching {
            XposedHelpers.findAndHookMethod(
                cmClass, "getActiveNetwork",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val n = param.result as? Network ?: return
                        val cm = param.thisObject as ConnectivityManager
                        val nc = cm.getNetworkCapabilities(n) ?: return
                        if (isVpnNetworkRaw(nc)) {
                            @Suppress("DEPRECATION")
                            val alt = cm.allNetworks.firstOrNull { candidate ->
                                val c = cm.getNetworkCapabilities(candidate) ?: return@firstOrNull false
                                !isVpnNetworkRaw(c)
                            }
                            param.result = alt
                        }
                    }
                }
            )
        }

        // getActiveNetworkInfo() → if VPN, substitute first non-VPN NetworkInfo.
        runCatching {
            XposedHelpers.findAndHookMethod(
                cmClass, "getActiveNetworkInfo",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val info = param.result as? NetworkInfo ?: return
                        if (info.type == TYPE_VPN) {
                            @Suppress("DEPRECATION")
                            val all = (param.thisObject as ConnectivityManager).allNetworkInfo
                            param.result = all.firstOrNull { it.isConnected && it.type != TYPE_VPN }
                        }
                    }
                }
            )
        }

        // getAllNetworkInfo() → strip VPN entries from the array.
        runCatching {
            XposedHelpers.findAndHookMethod(
                cmClass, "getAllNetworkInfo",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val arr = param.result as? Array<*> ?: return
                        val filtered = arr.filterIsInstance<NetworkInfo>()
                            .filter { it.type != TYPE_VPN }
                            .toTypedArray()
                        param.result = filtered
                    }
                }
            )
        }

        // getNetworkInfo(int type): return null for TYPE_VPN queries.
        runCatching {
            XposedHelpers.findAndHookMethod(
                cmClass, "getNetworkInfo", java.lang.Integer.TYPE,
                object : XC_MethodHook() {
                    override fun beforeHookedMethod(param: MethodHookParam) {
                        if ((param.args[0] as Int) == TYPE_VPN) param.result = null
                    }
                }
            )
        }

        // getNetworkInfo(Network): return null if the returned info is a VPN.
        runCatching {
            XposedHelpers.findAndHookMethod(
                cmClass, "getNetworkInfo", Network::class.java,
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val info = param.result as? NetworkInfo ?: return
                        if (info.type == TYPE_VPN) param.result = null
                    }
                }
            )
        }
    }

    // ------------------------------------------------------------------
    //  LinkProperties
    // ------------------------------------------------------------------

    private fun hookLinkProperties(cl: ClassLoader) {
        val lpClass = XposedHelpers.findClass("android.net.LinkProperties", cl)

        // getInterfaceName() — if it's a VPN tunnel, claim it's wlan0.
        val getIfaceName = lpClass.getDeclaredMethod("getInterfaceName")
        origGetInterfaceName = getIfaceName
        XposedBridge.hookMethod(getIfaceName, object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                val name = param.result as? String ?: return
                if (isVpnInterfaceName(name)) param.result = "wlan0"
            }
        })

        // getRoutes() — drop routes whose interface looks like a VPN tunnel.
        runCatching {
            XposedHelpers.findAndHookMethod(
                lpClass, "getRoutes",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val routes = param.result as? List<*> ?: return
                        val filtered = routes.filterNotNull().filter { route ->
                            val iface = runCatching {
                                XposedHelpers.callMethod(route, "getInterface") as? String
                            }.getOrNull()
                            iface == null || !isVpnInterfaceName(iface)
                        }
                        param.result = filtered
                    }
                }
            )
        }

        // getDnsServers() — if this LinkProperties belongs to a VPN
        // interface, return an empty list. We check the *real* (un-hooked)
        // interface name by invoking the original getInterfaceName()
        // directly, since our own hook above already rewrites it to
        // "wlan0" and we'd lose the ability to tell.
        runCatching {
            XposedHelpers.findAndHookMethod(
                lpClass, "getDnsServers",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        if (isVpnLinkProperties(param.thisObject)) {
                            param.result = emptyList<InetAddress>()
                        }
                    }
                }
            )
        }

        // getHttpProxy() — some network stacks surface a VPN-installed
        // HTTP proxy here. Drop it if the LinkProperties is a VPN one.
        runCatching {
            XposedHelpers.findAndHookMethod(
                lpClass, "getHttpProxy",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        if (isVpnLinkProperties(param.thisObject)) {
                            param.result = null
                        }
                    }
                }
            )
        }
    }

    /** Does this LinkProperties describe a VPN tunnel? Uses the original
     *  (un-hooked) getInterfaceName so we don't fool ourselves. */
    private fun isVpnLinkProperties(lp: Any?): Boolean {
        if (lp == null) return false
        val m = origGetInterfaceName ?: return false
        val rawName = try {
            XposedBridge.invokeOriginalMethod(m, lp, emptyArray<Any>()) as? String
        } catch (_: Throwable) {
            null
        } ?: return false
        return isVpnInterfaceName(rawName)
    }

    // ------------------------------------------------------------------
    //  java.net.NetworkInterface
    // ------------------------------------------------------------------

    private fun hookNetworkInterface() {
        val niClass = NetworkInterface::class.java

        // getNetworkInterfaces(): hide VPN tunnel interfaces from enumeration.
        XposedHelpers.findAndHookMethod(
            niClass, "getNetworkInterfaces",
            object : XC_MethodHook() {
                override fun afterHookedMethod(param: MethodHookParam) {
                    @Suppress("UNCHECKED_CAST")
                    val orig = param.result as? java.util.Enumeration<NetworkInterface> ?: return
                    val kept = mutableListOf<NetworkInterface>()
                    while (orig.hasMoreElements()) {
                        val iface = orig.nextElement()
                        if (!isVpnInterfaceName(iface.name) &&
                            !isVpnInterfaceName(iface.displayName ?: "")
                        ) {
                            kept.add(iface)
                        }
                    }
                    param.result = Collections.enumeration(kept)
                }
            }
        )

        // getByName(String): return null for VPN names.
        XposedHelpers.findAndHookMethod(
            niClass, "getByName", String::class.java,
            object : XC_MethodHook() {
                override fun beforeHookedMethod(param: MethodHookParam) {
                    val name = param.args[0] as? String ?: return
                    if (isVpnInterfaceName(name)) param.result = null
                }
            }
        )

        // getByIndex(int): drop result if the interface is a VPN tunnel.
        runCatching {
            XposedHelpers.findAndHookMethod(
                niClass, "getByIndex", java.lang.Integer.TYPE,
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val iface = param.result as? NetworkInterface ?: return
                        if (isVpnInterfaceName(iface.name)) param.result = null
                    }
                }
            )
        }

        // getByInetAddress(InetAddress): drop result if VPN.
        XposedHelpers.findAndHookMethod(
            niClass, "getByInetAddress", InetAddress::class.java,
            object : XC_MethodHook() {
                override fun afterHookedMethod(param: MethodHookParam) {
                    val iface = param.result as? NetworkInterface ?: return
                    if (isVpnInterfaceName(iface.name)) param.result = null
                }
            }
        )
    }

    // ------------------------------------------------------------------
    //  /proc/net/* file reads — redirect to /dev/null so reads yield empty
    // ------------------------------------------------------------------

    private fun hookProcNetFiles() {
        val devNull = "/dev/null"
        val devNullFile = File(devNull)

        // FileInputStream(String)
        XposedHelpers.findAndHookConstructor(
            FileInputStream::class.java, String::class.java,
            object : XC_MethodHook() {
                override fun beforeHookedMethod(param: MethodHookParam) {
                    val path = param.args[0] as? String ?: return
                    if (isSensitiveNetPath(path)) {
                        param.args[0] = devNull
                    }
                }
            }
        )

        // FileInputStream(File)
        XposedHelpers.findAndHookConstructor(
            FileInputStream::class.java, File::class.java,
            object : XC_MethodHook() {
                override fun beforeHookedMethod(param: MethodHookParam) {
                    val f = param.args[0] as? File ?: return
                    if (isSensitiveNetPath(f.absolutePath)) {
                        param.args[0] = devNullFile
                    }
                }
            }
        )

        // FileReader(String)
        runCatching {
            XposedHelpers.findAndHookConstructor(
                FileReader::class.java, String::class.java,
                object : XC_MethodHook() {
                    override fun beforeHookedMethod(param: MethodHookParam) {
                        val path = param.args[0] as? String ?: return
                        if (isSensitiveNetPath(path)) {
                            param.args[0] = devNull
                        }
                    }
                }
            )
        }

        // FileReader(File)
        runCatching {
            XposedHelpers.findAndHookConstructor(
                FileReader::class.java, File::class.java,
                object : XC_MethodHook() {
                    override fun beforeHookedMethod(param: MethodHookParam) {
                        val f = param.args[0] as? File ?: return
                        if (isSensitiveNetPath(f.absolutePath)) {
                            param.args[0] = devNullFile
                        }
                    }
                }
            )
        }
    }

    // ------------------------------------------------------------------
    //  System.getProperty — hide JVM-level proxy configuration
    // ------------------------------------------------------------------

    private fun hookSystemProperties() {
        // System.getProperty(String) and System.getProperty(String, String)
        // are the canonical way apps read http.proxyHost, socksProxyHost,
        // and friends. Both YourVPNDead and RKNHardering use these as VPN
        // signals. Hiding them by returning null (or the caller-provided
        // default) defeats that check without affecting non-proxy
        // properties.
        val hide = object : XC_MethodHook() {
            override fun beforeHookedMethod(param: MethodHookParam) {
                val key = param.args[0] as? String ?: return
                if (isProxyPropertyKey(key)) {
                    // Match the argument-count of the specific overload:
                    // 1-arg → null, 2-arg → the caller's default.
                    param.result = if (param.args.size == 2) param.args[1] else null
                }
            }
        }
        XposedHelpers.findAndHookMethod(
            System::class.java, "getProperty", String::class.java, hide
        )
        runCatching {
            XposedHelpers.findAndHookMethod(
                System::class.java, "getProperty",
                String::class.java, String::class.java, hide
            )
        }
    }

    private fun isProxyPropertyKey(key: String): Boolean = when (key) {
        "http.proxyHost", "http.proxyPort", "http.nonProxyHosts",
        "https.proxyHost", "https.proxyPort",
        "ftp.proxyHost", "ftp.proxyPort", "ftp.nonProxyHosts",
        "socksProxyHost", "socksProxyPort",
        "socks.proxyHost", "socks.proxyPort",
        "proxyHost", "proxyPort" -> true
        else -> false
    }

    // ------------------------------------------------------------------
    //  Helpers
    // ------------------------------------------------------------------

    private fun isVpnInterfaceName(name: String): Boolean {
        if (name.isEmpty()) return false
        val n = name.lowercase()
        return n.startsWith("tun") ||
            n.startsWith("ppp") ||
            n.startsWith("tap") ||
            n.startsWith("wg") ||
            n.startsWith("ipsec") ||
            n.startsWith("xfrm") ||
            n.startsWith("utun") ||
            n.startsWith("l2tp") ||
            n.startsWith("gre") ||
            n.contains("vpn")
    }

    private fun isSensitiveNetPath(path: String): Boolean {
        if (!path.startsWith("/proc/")) return false
        return when (path) {
            "/proc/net/route",
            "/proc/net/ipv6_route",
            "/proc/net/if_inet6",
            "/proc/net/tcp",
            "/proc/net/tcp6",
            "/proc/net/udp",
            "/proc/net/udp6",
            "/proc/net/dev",
            "/proc/net/arp",
            "/proc/net/route_cache",
            "/proc/net/rt_cache" -> true
            else -> path.startsWith("/proc/net/fib_trie") ||
                path.startsWith("/proc/net/fib_triestat") ||
                path.startsWith("/proc/net/xfrm_stat")
        }
    }

    companion object {
        // android.net.NetworkCapabilities.TRANSPORT_VPN
        private const val TRANSPORT_VPN = 4
        // android.net.NetworkCapabilities.NET_CAPABILITY_NOT_VPN
        private const val NET_CAPABILITY_NOT_VPN = 15
        // android.net.ConnectivityManager.TYPE_VPN
        private const val TYPE_VPN = 17
        // android.net.ConnectivityManager.TYPE_WIFI
        private const val TYPE_WIFI = 1
    }
}
