package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import android.os.Build
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.VpnPresence
import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.checks.NativeProbe
import dev.okhsunrog.vpnhide.suExec
import dev.okhsunrog.vpnhide.tunnelRouteProbeCommand
import dev.okhsunrog.vpnhide.tunnelRouteProbeResult
import java.io.File

/**
 * Runs the same native probes as **root** — uid 0 is not a hook target, so its
 * view is the unfiltered truth to diff against the app's in-process view.
 *
 * The probe binary ships as an APK asset (AGP 9 keeps native libs compressed in
 * the APK, so a jniLib isn't an on-disk executable). We extract it into the app
 * sandbox, then stage it to `/data/local/tmp` (correctly labelled for exec under
 * root) and run it via `su`.
 */
object GroundTruthProbe {
    // Pick the probe asset for the device's primary ABI: the APK bundles a
    // vhprobe per shipped ABI (arm64-v8a, armeabi-v7a). SUPPORTED_ABIS[0] is the
    // preferred one, so a 64-bit device gets the arm64 probe and a 32-bit-only
    // device gets the armv7 one. Fall back to arm64-v8a if the list is empty.
    private fun probeAsset(): String {
        val abi = Build.SUPPORTED_ABIS.firstOrNull() ?: "arm64-v8a"
        return "bin/$abi/vhprobe"
    }

    // Each entry point gets its own extraction dest and staged path so a
    // concurrent run() / selfRoutedThroughVpn() cannot cp-overwrite or rm each
    // other's binary mid-exec (which would spuriously fail one of them). prepare()
    // keeps the stable "vhprobe" name because its path is stored as the runtime
    // probe source and reused by the batched checks.
    private const val STAGED_ALL = "/data/local/tmp/vpnhide_vhprobe_all"
    private const val STAGED_UID = "/data/local/tmp/vpnhide_vhprobe_uid"
    private const val TAG = LogTags.DIAG

    /** id -> ground-truth outcome, or empty when root/exec is unavailable (the
     * caller then classifies those checks as NotMeasured(NoGroundTruth)). */
    fun run(context: Context): Map<String, CheckOutput> {
        val local = extractBinary(context, "vhprobe_all") ?: return emptyMap()
        val (exit, out) =
            suExec(
                "cp '${local.absolutePath}' $STAGED_ALL && chmod 700 $STAGED_ALL && $STAGED_ALL; rm -f $STAGED_ALL",
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
        val local = extractBinary(context, "vhprobe_uid") ?: return null
        val uid = android.os.Process.myUid()
        require(presence.interfaces.all { it.matches(Regex("[A-Za-z0-9_.:-]+")) })
        val interfaces = presence.interfaces.joinToString(",")
        val (exit, out) =
            suExec(
                "cp '${local.absolutePath}' $STAGED_UID && chmod 700 $STAGED_UID && " +
                    "$STAGED_UID --uid $uid --vpn-ifaces '$interfaces'; result=\$?; rm -f $STAGED_UID; exit \$result",
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

    /** Prepare the shared root-executable probe for batched runtime checks. */
    fun prepare(context: Context): File? = extractBinary(context, "vhprobe")

    private fun extractBinary(
        context: Context,
        name: String,
    ): File? =
        runCatching {
            val dest = File(context.filesDir, name)
            context.assets.open(probeAsset()).use { input -> dest.outputStream().use { input.copyTo(it) } }
            dest
        }.onFailure { VpnHideLog.w(TAG, "failed to extract vhprobe asset: ${it.message}") }
            .getOrNull()
}
