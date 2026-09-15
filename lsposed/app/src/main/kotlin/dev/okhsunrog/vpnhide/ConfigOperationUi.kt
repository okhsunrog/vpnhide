package dev.okhsunrog.vpnhide

import androidx.compose.foundation.layout.Row
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import dev.okhsunrog.vpnhide.ui.components.PreferenceRowSwitch
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch

@Composable
internal fun CanonicalPreferenceSwitch(
    field: CanonicalToggle,
    title: String,
    subtitle: String,
    icon: ImageVector,
    index: Int = -1,
    count: Int = 1,
    enabled: Boolean = true,
    write: suspend (Boolean) -> CanonicalWriteResult = { value ->
        CanonicalConfigRepository.commit(CanonicalMutation(listOf(CanonicalEdit.Toggle(field, value))))
    },
) {
    val state by CanonicalConfigRepository.state.collectAsState()
    val scope = rememberCoroutineScope()
    var requested by remember { mutableStateOf<Boolean?>(null) }
    val pending = state.pending[field]
    PreferenceRowSwitch(
        title = title,
        subtitle =
            if (field == CanonicalToggle.DebugSwitch && state.activeCaptures > 0) {
                subtitle + "\n" + stringResource(R.string.config_capture_logging)
            } else {
                subtitle
            },
        icon = icon,
        index = index,
        count = count,
        checked = requested ?: pending ?: state.confirmed?.let { canonicalToggle(it, field) } ?: false,
        enabled = enabled && state.mode == ConfigCoordinatorMode.Open && requested == null && pending == null,
        progress = requested != null || pending != null,
        onCheckedChange = { value ->
            requested = value
            scope.launch(start = CoroutineStart.UNDISPATCHED) {
                try {
                    write(value)
                } finally {
                    requested = null
                }
            }
        },
    )
}

@Composable
internal fun ConfigOperationStatus() {
    val view by CanonicalConfigRepository.state.collectAsState()
    val scope = rememberCoroutineScope()
    val status = configStatus(view) ?: return
    val message =
        when (status.message) {
            ConfigStatusMessage.Initializing -> R.string.config_initializing
            ConfigStatusMessage.RebootRequired -> R.string.config_reboot_required
            ConfigStatusMessage.Paused -> R.string.config_paused
            ConfigStatusMessage.Unavailable -> R.string.config_unavailable
            ConfigStatusMessage.Invalid -> R.string.config_invalid
            ConfigStatusMessage.Missing -> R.string.config_missing
            ConfigStatusMessage.Conflict -> R.string.config_ui_conflict
            ConfigStatusMessage.SecretFailed -> R.string.config_secret_failed
            ConfigStatusMessage.CleanupFailed -> R.string.config_cleanup_failed
            ConfigStatusMessage.ApplyFailed -> R.string.config_apply_failed
            ConfigStatusMessage.WriteFailed -> R.string.config_write_failed
            ConfigStatusMessage.Applying -> R.string.config_applying
            ConfigStatusMessage.Rechecking -> R.string.config_rechecking
        }
    StatusBanner(
        text = stringResource(message),
        containerColor = StatusColors.warningContainer(),
        contentColor = MaterialTheme.colorScheme.onSurface,
        action = {
            Row {
                if (status.canRecheck) {
                    TextButton(
                        onClick = CanonicalConfigRepository::retry,
                    ) { Text(stringResource(R.string.config_recheck)) }
                }
                if (status.canApply) {
                    TextButton(onClick = {
                        scope.launch { CanonicalConfigRepository.reconcile(ports = true) }
                    }) { Text(stringResource(R.string.config_apply_again)) }
                }
            }
        },
    )
}
