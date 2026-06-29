package dev.okhsunrog.vpnhide

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard

/**
 * Banner + retry button for the "VPN is not active, please turn it on and re-run
 * the checks" state. Used both on the Dashboard protection panel and the
 * Diagnostics screen so the UX is identical.
 */
@Composable
internal fun VpnOffPrompt(
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) = RetryPromptCard(R.string.vpn_off_prompt, onRetry, modifier)

/**
 * Banner + retry button for a diagnostics run that *failed* (root dropped, shell
 * exec error) rather than finding the VPN off — kept separate from
 * [VpnOffPrompt] so an active-VPN user isn't wrongly told their VPN is off.
 */
@Composable
internal fun DiagnosticsFailedPrompt(
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) = RetryPromptCard(R.string.diag_failed_prompt, onRetry, modifier)

@Composable
private fun RetryPromptCard(
    messageRes: Int,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) {
    EnhancedCard(
        modifier = modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surfaceVariant,
        contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                text = stringResource(messageRes),
                style = MaterialTheme.typography.bodyMedium,
            )
            Spacer(Modifier.height(12.dp))
            EnhancedButton(
                onClick = onRetry,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(stringResource(R.string.vpn_off_retry))
            }
        }
    }
}
