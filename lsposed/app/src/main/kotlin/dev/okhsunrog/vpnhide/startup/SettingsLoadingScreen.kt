package dev.okhsunrog.vpnhide.startup

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.ui.theme.VpnHideTheme

/** No editable controls or settings-driven effects until DataStore supplied a real snapshot. */
@Composable
internal fun SettingsLoadingScreen(
    error: String?,
    retry: () -> Unit,
) {
    VpnHideTheme {
        if (error != null) {
            StartupBlockingScreen(
                title = stringResource(R.string.settings_load_failed_title),
                body = stringResource(R.string.settings_load_failed_body),
                actionLabel = stringResource(R.string.vpn_off_retry),
                onAction = retry,
            )
        } else {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
        }
    }
}
