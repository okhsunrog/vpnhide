package dev.okhsunrog.vpnhide

import android.content.Context

internal object AgentControlBridge {
    @Suppress("UNUSED_PARAMETER")
    suspend fun setEnabled(
        context: Context,
        enabled: Boolean,
    ) {
        // The host bridge is debug-only and is not present in release builds.
    }
}
