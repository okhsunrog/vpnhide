package dev.okhsunrog.vpnhide

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.filled.BugReport
import androidx.compose.material.icons.filled.Forum
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.ui.components.EnhancedOutlinedButton
import dev.okhsunrog.vpnhide.ui.components.PreferenceRow

// Author / community contacts. Centralised so the dashboard banner action and
// the Settings entry open the exact same set of links.
internal const val CONTACT_GITHUB_ISSUES_URL = "https://github.com/okhsunrog/vpnhide/issues"
internal const val CONTACT_TELEGRAM_URL = "https://t.me/+ptRzrpTkt0ViODdi"
internal const val CONTACT_4PDA_URL = "https://4pda.to/forum/index.php?showtopic=1120926"

/**
 * Shared "Community & feedback" modal — three external links (GitHub issues,
 * Telegram group, 4PDA thread). Opened from the dashboard's experimental-backend
 * banner and from Settings; both render this one composable so the contact set
 * can't drift between entry points.
 */
@Composable
internal fun ContactModal(onDismiss: () -> Unit) {
    val uriHandler = LocalUriHandler.current

    fun open(url: String) {
        runCatching {
            uriHandler.openUri(url)
        }
        onDismiss()
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.contact_title)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(
                    text = stringResource(R.string.contact_body),
                    style = MaterialTheme.typography.bodyMedium,
                )
                PreferenceRow(
                    title = stringResource(R.string.contact_github),
                    subtitle = stringResource(R.string.contact_github_sub),
                    icon = Icons.Default.BugReport,
                    onClick = { open(CONTACT_GITHUB_ISSUES_URL) },
                )
                PreferenceRow(
                    title = stringResource(R.string.contact_telegram),
                    subtitle = stringResource(R.string.contact_telegram_sub),
                    icon = Icons.AutoMirrored.Filled.Chat,
                    onClick = { open(CONTACT_TELEGRAM_URL) },
                )
                PreferenceRow(
                    title = stringResource(R.string.contact_4pda),
                    subtitle = stringResource(R.string.contact_4pda_sub),
                    icon = Icons.Default.Forum,
                    onClick = { open(CONTACT_4PDA_URL) },
                )
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.contact_close)) }
        },
    )
}

/** Banner action button that opens [ContactModal]. */
@Composable
internal fun ContactAuthorButton(onClick: () -> Unit) {
    EnhancedOutlinedButton(onClick = onClick) {
        Text(stringResource(R.string.contact_button))
    }
}
