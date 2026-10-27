package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.checks.AppVpnStateObservation
import dev.okhsunrog.vpnhide.checks.AppVpnStateResponse
import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.checks.ChecksResponse
import dev.okhsunrog.vpnhide.checks.NativeProbe
import dev.okhsunrog.vpnhide.core.CoreRoot
import uniffi.vpnhide_core_ffi.CoreDiagnosticProbe as CoreProbe

/**
 * Runs the same native probes as **root** — uid 0 is not a hook target, so its
 * view is the unfiltered truth to diff against the app's in-process view. Core runs
 * them (`vpnhide-core probe checks` / `observe app-vpn-state`) as typed diagnostics
 * probes; the app runs no root process.
 */
object GroundTruthProbe {
    private const val TAG = LogTags.DIAG

    /** id -> ground-truth outcome, or empty when core is unavailable (the
     * caller then classifies those checks as NotMeasured(NoGroundTruth)). */
    fun run(context: Context): Map<String, CheckOutput> {
        CoreRoot.attach(context)
        val result = CoreRoot.collectOneBlocking(CoreProbe.GroundTruthChecks)
        if (!result.ok) {
            VpnHideLog.w(TAG, "ground-truth probe unavailable (core exit=${result.exit})")
            return emptyMap()
        }
        return when (val response = NativeProbe.parseChecks(result.text.trim())) {
            is ChecksResponse.Success -> {
                response.checks
            }

            is ChecksResponse.Failure -> {
                VpnHideLog.w(TAG, "ground-truth probe rejected: ${response.error}")
                emptyMap()
            }
        }
    }

    /** One hook-inert root observation of VPN presence and this app's UID routing. */
    internal fun observeAppVpnState(context: Context): AppVpnStateObservation? {
        CoreRoot.attach(context)
        val result =
            CoreRoot.collectOneBlocking(
                CoreProbe.AppVpnState(
                    android.os.Process
                        .myUid()
                        .toUInt(),
                ),
            )
        if (!result.ok) {
            VpnHideLog.w(TAG, "app VPN state unavailable (core exit=${result.exit})")
            return null
        }
        return when (val response = NativeProbe.parseAppVpnState(result.text.trim())) {
            is AppVpnStateResponse.Success -> {
                response.observation
            }

            is AppVpnStateResponse.Failure -> {
                VpnHideLog.w(TAG, "app VPN state observation rejected: ${response.error}")
                null
            }
        }
    }
}
