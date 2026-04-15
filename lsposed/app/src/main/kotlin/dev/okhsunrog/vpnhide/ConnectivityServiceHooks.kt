package dev.okhsunrog.vpnhide

import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import android.os.Binder
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers

/**
 * Server-side ConnectivityService hooks.
 *
 * Replaces the writeToParcel-based approach in HookEntry. Rationale and
 * roadmap are documented in the PR description; in short:
 *
 *   - writeToParcel runs in many contexts (sticky broadcasts, callback
 *     fan-out, internal persistence) where Binder.getCallingUid() does
 *     NOT correspond to the recipient of the parcel. That is brittle
 *     and may leak modifications to non-target subscribers.
 *
 *   - Hooking the server-side getters in ConnectivityService is the
 *     standard pattern: each getter runs inside the incoming Binder
 *     transaction from the requesting app, so getCallingUid() is the
 *     real caller, period.
 *
 * Status: SKELETON. This file installs hooks but the per-method handlers
 * are not yet wired to actual filtering — see TODO blocks below. The old
 * writeToParcel hooks in HookEntry remain active for the time being so
 * functionality is not regressed during the migration.
 */
internal object ConnectivityServiceHooks {
    private const val SERVICE_CLASS = "com.android.server.ConnectivityService"

    /**
     * Methods on ConnectivityService that return network state and that
     * we want to filter for target callers. The list is ordered roughly
     * from "most commonly called by apps" to "edge cases".
     *
     * Names only — XposedHelpers.findAndHookMethod resolves overloads at
     * install time. Some of these have multiple signatures across AOSP
     * versions (Android 11/12/13/14/15); we hook by name and let the
     * handler inspect args at runtime where needed.
     */
    private val TARGET_METHODS =
        listOf(
            // NetworkInfo getters — legacy API but still widely used by
            // older apps and SDKs (AppsFlyer, Adjust, banking SDKs).
            "getActiveNetworkInfo",
            "getActiveNetworkInfoForUid",
            "getNetworkInfo",
            "getNetworkInfoForUid",
            "getAllNetworkInfo",
            // Network handle getters — used to look up a Network object,
            // which is then passed to getNetworkCapabilities/getLinkProperties.
            "getActiveNetwork",
            "getActiveNetworkForUid",
            "getAllNetworks",
            // NetworkCapabilities + LinkProperties — modern API surface,
            // primary path on Android 11+.
            "getNetworkCapabilities",
            "getLinkProperties",
            "getActiveLinkProperties",
            // Default network capabilities by user — used by some
            // ConnectivityManager.getDefaultNetworkCapabilitiesForUser
            // wrappers; rarely invoked but cheap to cover.
            "getDefaultNetworkCapabilitiesForUser",
        )

    fun install(
        classLoader: ClassLoader,
        isTargetCaller: () -> Boolean,
        sanitizeNetworkCapabilities: (NetworkCapabilities) -> NetworkCapabilities?,
        sanitizeNetworkInfo: (NetworkInfo) -> NetworkInfo?,
        sanitizeLinkProperties: (LinkProperties) -> LinkProperties?,
    ) {
        val cls =
            try {
                XposedHelpers.findClass(SERVICE_CLASS, classLoader)
            } catch (t: Throwable) {
                XposedBridge.log("VpnHide: ConnectivityService class not found: ${t.message}")
                return
            }

        var hookedCount = 0
        for (methodName in TARGET_METHODS) {
            // Hook ALL overloads of the method — across AOSP versions
            // signatures vary, and some methods have @hide variants.
            val methods =
                cls.declaredMethods.filter { it.name == methodName }
            if (methods.isEmpty()) {
                // Method not present on this AOSP version — fine, skip.
                continue
            }
            for (m in methods) {
                try {
                    XposedBridge.hookMethod(
                        m,
                        object : XC_MethodHook() {
                            override fun afterHookedMethod(param: MethodHookParam) {
                                // Bail fast for non-target callers — keep
                                // hot path under a single Set.contains().
                                if (!isTargetCaller()) return

                                val original = param.result ?: return
                                val sanitized: Any? =
                                    when (original) {
                                        is NetworkCapabilities -> {
                                            sanitizeNetworkCapabilities(original)
                                        }

                                        is NetworkInfo -> {
                                            sanitizeNetworkInfo(original)
                                        }

                                        is LinkProperties -> {
                                            sanitizeLinkProperties(original)
                                        }

                                        is Array<*> -> {
                                            sanitizeArray(
                                                original,
                                                sanitizeNetworkCapabilities,
                                                sanitizeNetworkInfo,
                                                sanitizeLinkProperties,
                                            )
                                        }

                                        is Network -> {
                                            // TODO(refactor): hide VPN Network handle entirely
                                            // by returning null when the underlying NC has
                                            // TRANSPORT_VPN. Needs cross-call lookup of NC by
                                            // Network handle — wire up later.
                                            null
                                        }

                                        else -> {
                                            null
                                        }
                                    }
                                if (sanitized != null) {
                                    param.result = sanitized
                                }
                            }
                        },
                    )
                    hookedCount++
                } catch (t: Throwable) {
                    XposedBridge.log(
                        "VpnHide: failed to hook ConnectivityService.$methodName " +
                            "(${m.parameterTypes.joinToString { it.simpleName }}): ${t.message}",
                    )
                }
            }
        }
        XposedBridge.log("VpnHide: installed $hookedCount ConnectivityService hooks")
    }

    /**
     * Sanitize an array result (e.g. getAllNetworkInfo, getAllNetworks).
     * Returns a new array with VPN entries removed/sanitized, or null if
     * no changes were needed.
     */
    private fun sanitizeArray(
        original: Array<*>,
        sanitizeNc: (NetworkCapabilities) -> NetworkCapabilities?,
        sanitizeNi: (NetworkInfo) -> NetworkInfo?,
        sanitizeLp: (LinkProperties) -> LinkProperties?,
    ): Array<*>? {
        // TODO(refactor): implement per-element-type filtering.
        // For NetworkInfo[]: drop VPN entries entirely.
        // For Network[]: filter out handles that resolve to VPN networks.
        // For NetworkCapabilities[]: sanitize each element.
        return null
    }

    /**
     * Marker for the "shall I touch this caller" check. Provided by
     * HookEntry so the cache invalidation path stays in one place.
     */
    @Suppress("unused")
    private fun callerIsTarget(): Boolean =
        false.also {
            // Placeholder; real impl is injected via the lambda passed to
            // install(). Kept here so future readers can grep the file for
            // the concept.
            Binder.getCallingUid()
        }
}
