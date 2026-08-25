package dev.okhsunrog.vpnhide

import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
import dev.okhsunrog.vpnhide.diagnostics.Verdict
import dev.okhsunrog.vpnhide.diagnostics.verdict
import dev.okhsunrog.vpnhide.ui.components.EnhancedOutlinedButton

/**
 * How long after install the donation prompt stays quiet.
 *
 * The first days are when the user is still finding out whether hiding works at
 * all — wrong kernel, wrong root manager, module not loading — and when most of
 * the people who will uninstall do. Asking then is asking on credit.
 */
private const val DONATE_PROMPT_MIN_AGE_MILLIS = 7L * 24 * 60 * 60 * 1000

/**
 * Whether the Dashboard should show the donation banner.
 *
 * Deliberately strict: we ask only when the app has *demonstrably* done its job
 * on this device — a diagnostics run that wasn't gated (VPN up, self routed, no
 * pending restart) with every layer active and leaking nothing, and no issues
 * on screen to sit next to. Anything less and the banner would be asking for
 * money over a broken setup.
 *
 * Pure so the gate is readable in one place; the caller supplies the clock.
 */
internal fun shouldShowDonatePrompt(
    dismissed: Boolean,
    installedAtMillis: Long,
    nowMillis: Long,
    protection: ProtectionCheck,
    hasIssues: Boolean,
): Boolean {
    if (dismissed || hasIssues) return false
    if (nowMillis - installedAtMillis < DONATE_PROMPT_MIN_AGE_MILLIS) return false
    val checked = protection as? ProtectionCheck.Checked ?: return false
    return listOf(checked.native, checked.java).all { layer ->
        layer is LayerStatus.Active && layer.verdict == Verdict.Ok
    }
}

/**
 * Install time of this app, used as the prompt's age reference — no separate
 * "first run" timestamp to persist. Survives updates and resets on reinstall.
 * Falls back to [nowMillis] (age 0 → no prompt) if the lookup fails.
 */
internal fun appInstalledAtMillis(
    context: Context,
    nowMillis: Long,
): Long {
    val installed =
        runCatching {
            context.packageManager.getPackageInfo(context.packageName, 0).firstInstallTime
        }.getOrNull()
    return installed?.takeIf { it > 0 } ?: nowMillis
}

/**
 * The Dashboard donation banner: one line of text, "Support" and "Hide".
 *
 * Both buttons dismiss for good — [onDonate] opens the same [DonateModal] the
 * Settings entry uses, and someone who opened it and walked away doesn't need
 * to be asked again. There is no "later": a prompt that comes back is a nag,
 * and Settings keeps a permanent entry for anyone who changes their mind.
 */
@Composable
internal fun DonatePromptBanner(
    containerColor: Color,
    contentColor: Color,
    onDonate: () -> Unit,
    onHide: () -> Unit,
) {
    StatusBanner(
        text = stringResource(R.string.donate_banner),
        containerColor = containerColor,
        contentColor = contentColor,
        action = {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                EnhancedOutlinedButton(onClick = onDonate) {
                    Text(stringResource(R.string.donate_banner_support))
                }
                EnhancedOutlinedButton(onClick = onHide) {
                    Text(stringResource(R.string.donate_banner_hide))
                }
            }
        },
    )
}
