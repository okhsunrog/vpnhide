package dev.okhsunrog.vpnhide

import android.net.LinkProperties
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers

/**
 * Server-side ConnectivityService hooks.
 *
 * Replaces the writeToParcel-based approach in HookEntry. Rationale:
 *
 *   - writeToParcel runs in many contexts (sticky broadcasts, callback
 *     fan-out, internal persistence) where `Binder.getCallingUid()`
 *     does NOT correspond to the recipient of the parcel. That's
 *     brittle and may leak modifications to non-target subscribers.
 *
 *   - Hooking the server-side getters in ConnectivityService is the
 *     standard pattern: each getter runs inside the incoming Binder
 *     transaction from the requesting app, so `getCallingUid()` is
 *     the real caller, period.
 *
 * The `NetworkSanitizers` object provides the pure sanitization logic;
 * this file only handles hook installation and result dispatch.
 *
 * HookEntry keeps the writeToParcel fallback active in parallel for
 * callback/broadcast paths these server-side hooks don't cover.
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
                                            )
                                        }

                                        // Network handles are opaque references;
                                        // the sanitization happens when the caller
                                        // queries getNetworkCapabilities(handle) /
                                        // getLinkProperties(handle), both of which
                                        // we also hook. No need to swap the handle.
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
     * Sanitize an array result. Covers:
     *
     *   - `NetworkInfo[]`           from `getAllNetworkInfo`
     *   - `NetworkCapabilities[]`   from `getDefaultNetworkCapabilitiesForUser`
     *
     * `Network[]` is deliberately left untouched — handles are opaque,
     * filtering happens at the follow-up `getNetworkCapabilities(handle)`
     * / `getLinkProperties(handle)` call.
     *
     * Array length is preserved and VPN elements are replaced with their
     * sanitized copies (WIFI-disguised for NetworkInfo, VPN bits cleared
     * for NetworkCapabilities). Preserving the length matches the legacy
     * writeToParcel semantics — callers that index into the array or use
     * `.size` for UI/state decisions see no surprise.
     *
     * Returns a new array with the substitutions applied, or null when
     * nothing was changed (caller passes through the original).
     */
    private fun sanitizeArray(
        original: Array<*>,
        sanitizeNc: (NetworkCapabilities) -> NetworkCapabilities?,
        sanitizeNi: (NetworkInfo) -> NetworkInfo?,
    ): Array<*>? {
        if (original.isEmpty()) return null
        val componentType = original.javaClass.componentType ?: return null

        var modified = false
        val replacements = arrayOfNulls<Any?>(original.size)
        for ((i, element) in original.withIndex()) {
            val sanitized: Any? =
                when (element) {
                    is NetworkInfo -> sanitizeNi(element)
                    is NetworkCapabilities -> sanitizeNc(element)
                    else -> null
                }
            if (sanitized != null) {
                replacements[i] = sanitized
                modified = true
            } else {
                replacements[i] = element
            }
        }
        if (!modified) return null

        // Build a result array of the correct component type so Android
        // APIs that cast the Binder reply back to `NetworkInfo[]` etc.
        // don't hit ClassCastException.
        @Suppress("UNCHECKED_CAST")
        val result =
            java.lang.reflect.Array
                .newInstance(componentType, original.size) as Array<Any?>
        for (i in original.indices) result[i] = replacements[i]
        return result
    }
}
