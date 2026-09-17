package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.appHelperStagedPath
import dev.okhsunrog.vpnhide.buildAppHelperStageCommand
import dev.okhsunrog.vpnhide.checks.AppVpnStateObservation
import dev.okhsunrog.vpnhide.checks.AppVpnStateResponse
import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.checks.NativeProbe
import dev.okhsunrog.vpnhide.extractAppHelper
import dev.okhsunrog.vpnhide.suExec
import java.io.File

/**
 * Runs the same native probes as **root** — uid 0 is not a hook target, so its
 * view is the unfiltered truth to diff against the app's in-process view.
 *
 * The helper binary ships as an APK asset (AGP 9 keeps native libs compressed in
 * the APK, so a jniLib isn't an on-disk executable). We extract it into the app
 * sandbox, then stage it to `/data/local/tmp` (correctly labelled for exec under
 * root) and run it via `su`.
 */
object GroundTruthProbe {
    private const val TAG = LogTags.DIAG

    /** id -> ground-truth outcome, or empty when root/exec is unavailable (the
     * caller then classifies those checks as NotMeasured(NoGroundTruth)). */
    fun run(context: Context): Map<String, CheckOutput> {
        val asset = extractAppHelper(context) ?: return emptyMap()
        val staged = appHelperStagedPath(asset.digest)
        val (exit, out) =
            suExec(
                buildAppHelperStageCommand(asset) +
                    " && $staged probe checks; result=\$?; exit \$result",
            )
        val json = out.trim()
        if (exit != 0) {
            VpnHideLog.w(TAG, "ground-truth probe unavailable (exit=$exit, no root?)")
            return emptyMap()
        }
        return when (val response = NativeProbe.parseChecks(json)) {
            is dev.okhsunrog.vpnhide.checks.ChecksResponse.Success -> {
                response.checks
            }

            is dev.okhsunrog.vpnhide.checks.ChecksResponse.Failure -> {
                VpnHideLog.w(TAG, "ground-truth probe rejected: ${response.error}")
                emptyMap()
            }
        }
    }

    /** One hook-inert root observation of VPN presence and this app's UID routing. */
    internal fun observeAppVpnState(context: Context): AppVpnStateObservation? {
        val asset = extractAppHelper(context) ?: return null
        val staged = appHelperStagedPath(asset.digest)
        val uid = android.os.Process.myUid()
        val (exit, out) =
            suExec(
                buildAppHelperStageCommand(asset) +
                    " && $staged observe app-vpn-state --uid $uid; result=\$?; exit \$result",
            )
        val json = out.trim()
        if (exit != 0) {
            VpnHideLog.w(TAG, "app VPN state unavailable (exit=$exit, no root?)")
            return null
        }
        return when (val response = NativeProbe.parseAppVpnState(json)) {
            is AppVpnStateResponse.Success -> {
                response.observation
            }

            is AppVpnStateResponse.Failure -> {
                VpnHideLog.w(TAG, "app VPN state observation rejected: ${response.error}")
                null
            }
        }
    }

    /** Prepare the shared app-private helper source for the batched root snapshot. */
    fun prepare(context: Context): File? = extractAppHelper(context)?.local
}
