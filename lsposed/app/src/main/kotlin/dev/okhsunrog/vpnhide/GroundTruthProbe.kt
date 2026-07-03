package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.checks.NativeProbe
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
    private const val ASSET = "bin/arm64-v8a/vhprobe"
    private const val STAGED = "/data/local/tmp/vpnhide_vhprobe"
    private const val TAG = LogTags.DIAG

    /** id -> ground-truth outcome, or empty when root/exec is unavailable (the
     * caller then classifies those checks as NotMeasured(NoGroundTruth)). */
    fun run(context: Context): Map<String, CheckOutput> {
        val local = extractBinary(context) ?: return emptyMap()
        val (exit, out) =
            suExec(
                "cp '${local.absolutePath}' $STAGED && chmod 700 $STAGED && $STAGED; rm -f $STAGED",
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
     * policy rules). Returns null when root/exec is unavailable — the caller then
     * does not block, since a no-root device is already handled as "VPN off".
     */
    fun selfRoutedThroughVpn(context: Context): Boolean? {
        val local = extractBinary(context) ?: return null
        val uid = android.os.Process.myUid()
        val (exit, out) =
            suExec(
                "cp '${local.absolutePath}' $STAGED && chmod 700 $STAGED && $STAGED --uid $uid; rm -f $STAGED",
            )
        val json = out.trim()
        if (exit != 0 || !json.startsWith("{")) {
            VpnHideLog.w(TAG, "self-routed probe unavailable (exit=$exit, no root?)")
            return null
        }
        return runCatching { org.json.JSONObject(json).getBoolean("routed") }.getOrNull()
    }

    private fun extractBinary(context: Context): File? =
        runCatching {
            val dest = File(context.filesDir, "vhprobe")
            context.assets.open(ASSET).use { input -> dest.outputStream().use { input.copyTo(it) } }
            dest
        }.onFailure { VpnHideLog.w(TAG, "failed to extract vhprobe asset: ${it.message}") }
            .getOrNull()
}
