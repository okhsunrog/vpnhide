package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.VpnPresence
import dev.okhsunrog.vpnhide.appHelperStagedPath
import dev.okhsunrog.vpnhide.buildAppHelperStageCommand
import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.checks.NativeProbe
import dev.okhsunrog.vpnhide.extractAppHelper
import dev.okhsunrog.vpnhide.suExec
import dev.okhsunrog.vpnhide.tunnelRouteProbeCommand
import dev.okhsunrog.vpnhide.tunnelRouteProbeResult
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
        if (exit != 0 || !json.startsWith("[")) {
            VpnHideLog.w(TAG, "ground-truth probe unavailable (exit=$exit, no root?)")
            return emptyMap()
        }
        return NativeProbe.parse(json)
    }

    /**
     * The self-in-tunnel gate: is this app's own uid routed through the VPN?
     * Runs the probe as root with `--uid` (a hook-inert, unfiltered read of the
     * policy rules). Returns null when root/exec or routing evidence is unavailable;
     * the caller reports an inconclusive check instead of blaming split tunneling.
     */
    internal fun selfRoutedThroughVpn(
        context: Context,
        presence: VpnPresence,
        sections: Map<String, String>,
    ): Boolean? {
        val asset = extractAppHelper(context) ?: return null
        val staged = appHelperStagedPath(asset.digest)
        val uid = android.os.Process.myUid()
        require(presence.interfaces.all { it.matches(Regex("[A-Za-z0-9_.:-]+")) })
        val interfaces = presence.interfaces.joinToString(",")
        val (exit, out) =
            suExec(
                buildAppHelperStageCommand(asset) +
                    " && $staged probe routing --uid $uid --vpn-ifaces '$interfaces'; result=\$?; exit \$result",
            )
        val json = out.trim()
        if (exit != 0 || !json.startsWith("{")) {
            VpnHideLog.w(TAG, "self-routed probe unavailable (exit=$exit, no root?)")
            return null
        }
        val routed = runCatching { org.json.JSONObject(json).getBoolean("routed") }.getOrNull()
        if (routed != false) return routed
        val unmanaged = presence.interfaces - presence.frameworkInterfaces
        if (unmanaged.isEmpty()) return false
        val command = tunnelRouteProbeCommand(sections, unmanaged, uid)
        if (command.isBlank()) return null
        val (routeExit, routes) = suExec(command)
        return if (routeExit == 0) tunnelRouteProbeResult(routes, unmanaged) else null
    }

    /** Prepare the shared app-private helper source for the batched root snapshot. */
    fun prepare(context: Context): File? = extractAppHelper(context)?.local
}
