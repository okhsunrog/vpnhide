package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.hook.HookLog

/**
 * Single registry of the logcat tags this project emits, so call sites, the
 * [HookLog] system_server sink and the debug-capture logcat filter
 * ([LogcatRecorder]) all agree on one spelling instead of re-typing string
 * literals. Native-backend tags (kmod / ports / zygisk / shadowhook) are listed
 * too — the app never logs under them, but the bundle filter needs them.
 */
internal object LogTags {
    /** Generic app-process tag (shell helpers, LSPosed state writes). */
    const val APP = "VpnHide"
    const val STARTUP = "VpnHide-Startup"
    const val DASHBOARD = "VpnHide-Dashboard"

    /** system_server hooks ([HookLog]). */
    const val LSPOSED = "VpnHide-LSPosed"
    const val DIAG = "VpnHide-Diag"
    const val LOGCAT = "VpnHide-Logcat"
    const val UPDATE = "VpnHide-Update"
    const val TARGETS = "VpnHide-Targets"
    const val STATISTICS = "VpnHide-Statistics"
    const val APP_LIST = "VpnHide-AppList"
    const val DEBUG_CONFIG = "VpnHideDebugConfig"
    const val AGENT_BRIDGE = "VpnHideAgentBridge"

    /** Diagnostics self-test tag (distinct capitalisation, matched separately). */
    const val TEST = "VPNHideTest"

    /** The Rust probe crate: panic reports from its own hook, and the Kotlin
     *  side's report of a probe run that threw. */
    const val NATIVE = "VpnHide-Native"

    /** App-process (and system_server) tags captured in debug bundles. */
    val APP_TAGS =
        listOf(
            TEST,
            APP,
            STARTUP,
            DASHBOARD,
            LSPOSED,
            DIAG,
            LOGCAT,
            UPDATE,
            AGENT_BRIDGE,
            TARGETS,
            STATISTICS,
            APP_LIST,
            DEBUG_CONFIG,
            NATIVE,
        )

    /** Native-backend logcat tags (kmod / ports / zygisk / shadowhook). */
    val NATIVE_TAGS =
        listOf(
            "vpnhide",
            "vpnhide_ports",
            "vpnhide-zygisk",
            "shadowhook_tag",
        )
}
