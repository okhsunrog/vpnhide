package dev.okhsunrog.vpnhide

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.compose.LocalLifecycleOwner
import dev.okhsunrog.vpnhide.ui.components.ButtonSpinner
import kotlinx.coroutines.launch

internal val LocalConfigSnackbar = staticCompositionLocalOf<SnackbarHostState> { error("Missing configuration feedback host") }

/** A rejected UI action explains the gate without submitting or replaying a mutation. */
internal val LocalConfigWriteAccess = staticCompositionLocalOf<() -> Boolean> { error("Missing configuration feedback host") }

/** Retain acknowledgement through Activity recreation, but never across a new coordinator process. */
internal class ConfigFeedbackViewModel : ViewModel() {
    var dialog by mutableStateOf(ConfigDialogState())
    var notifiedNotice: ConfigFailureNotice? = null
}

/** One owner across navigation; notifications overlay content instead of changing its height. */
@Composable
internal fun ConfigFeedbackProvider(content: @Composable () -> Unit) {
    val view by CanonicalConfigRepository.state.collectAsState()
    val host = remember { SnackbarHostState() }
    val owner = LocalLifecycleOwner.current as ViewModelStoreOwner
    val feedback = ViewModelProvider(owner)[ConfigFeedbackViewModel::class.java]

    LaunchedEffect(view.mode, view.rechecking) {
        feedback.dialog = reduceConfigDialog(feedback.dialog, ConfigDialogEvent.Observed(view.mode, view.rechecking))
    }
    ConfigFailureSnackbar(view, host, feedback)
    val checkWrite =
        remember(feedback) {
            {
                val mode = CanonicalConfigRepository.state.value.mode
                feedback.dialog = reduceConfigDialog(feedback.dialog, ConfigDialogEvent.WriteAttempted(mode))
                mode == ConfigCoordinatorMode.Open
            }
        }
    CompositionLocalProvider(
        LocalConfigSnackbar provides host,
        LocalConfigWriteAccess provides checkWrite,
    ) {
        Box(Modifier.fillMaxSize()) {
            content()
            SnackbarHost(
                hostState = host,
                modifier =
                    Modifier
                        .align(Alignment.BottomCenter)
                        .imePadding()
                        .navigationBarsPadding()
                        .padding(12.dp),
            )
        }
    }
    if (feedback.dialog.visible && view.mode != ConfigCoordinatorMode.Open) {
        ConfigBlockedDialog(view) {
            feedback.dialog = reduceConfigDialog(feedback.dialog, ConfigDialogEvent.Dismissed(view.mode))
        }
    }
}

@Composable
private fun ConfigFailureSnackbar(
    view: ConfigCoordinatorView,
    host: SnackbarHostState,
    feedback: ConfigFeedbackViewModel,
) {
    val notice = configFailureNotice(view)
    val message = notice?.status?.let { stringResource(configStatusText(it.message)) }
    val action = if (notice?.status?.canApply == true) stringResource(R.string.config_apply_again) else null
    val scope = rememberCoroutineScope()
    LaunchedEffect(notice) {
        if (notice == null || message == null || feedback.notifiedNotice == notice) return@LaunchedEffect
        feedback.notifiedNotice = notice
        val result = host.showSnackbar(message, actionLabel = action, withDismissAction = true, duration = SnackbarDuration.Long)
        if (result == SnackbarResult.ActionPerformed) {
            // The provider remains composed while the new operation replaces this notice.
            scope.launch { CanonicalConfigRepository.reconcile(ports = true) }
        }
    }
}

@Composable
private fun ConfigBlockedDialog(
    view: ConfigCoordinatorView,
    onDismiss: () -> Unit,
) {
    val status = configStatus(view) ?: return
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.config_changes_unavailable)) },
        text = {
            Column {
                Text(stringResource(configStatusText(status.message)))
                if (view.rechecking || view.mode == ConfigCoordinatorMode.Initializing) {
                    ButtonSpinner(Modifier.padding(top = 12.dp))
                }
            }
        },
        confirmButton = {
            if (status.canRecheck) {
                TextButton(onClick = CanonicalConfigRepository::retry) { Text(stringResource(R.string.config_recheck)) }
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.config_close)) }
        },
    )
}

private fun configStatusText(message: ConfigStatusMessage): Int =
    when (message) {
        ConfigStatusMessage.Initializing -> R.string.config_initializing
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
