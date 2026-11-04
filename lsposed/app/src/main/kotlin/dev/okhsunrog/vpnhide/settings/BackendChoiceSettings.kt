package dev.okhsunrog.vpnhide.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.CanonicalMutation
import dev.okhsunrog.vpnhide.CoreSessions
import dev.okhsunrog.vpnhide.LocalConfigWriteAccess
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.core.CoreRootEffect
import dev.okhsunrog.vpnhide.ui.components.PreferenceRow
import kotlinx.coroutines.launch
import uniffi.vpnhide_core_ffi.CoreBackendChoice
import uniffi.vpnhide_core_ffi.CoreBackendChoiceRequest
import uniffi.vpnhide_core_ffi.CoreBackendChoiceStage
import uniffi.vpnhide_core_ffi.CoreBackendChoiceStatus
import uniffi.vpnhide_core_ffi.CoreNativeBackend

/** Thin device-local intent adapter. Core owns applicability, staging and receipts. */
@Composable
internal fun BackendChoiceSettingsSection() {
    val core by CoreSessions.updates.collectAsState()
    val status = core.state?.backendChoice ?: return
    var open by remember { mutableStateOf(false) }
    var submitting by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val checkWrite = LocalConfigWriteAccess.current
    PreferenceRow(
        title = stringResource(R.string.backend_choice_title),
        subtitle = "${backendChoiceLabel(status.desired)} · ${backendChoiceStageLabel(status.stage)}",
        icon = Icons.Default.Settings,
        index = 0,
        count = 1,
        enabled = core.current && status.options.isNotEmpty() && !submitting,
        onClick = { if (checkWrite()) open = true },
    )
    if (open) {
        BackendChoiceDialog(status, onDismiss = { open = false }) { choice ->
            if (checkWrite()) {
                open = false
                submitting = true
                scope.launch {
                    try {
                        CanonicalConfigRepository.commit(
                            CanonicalMutation(
                                emptyList(),
                                coupledEffects =
                                    listOf(
                                        CoreRootEffect.StageBackendChoice(CoreBackendChoiceRequest(status.revision, choice, null)),
                                    ),
                                protectAllDrafts = true,
                            ),
                        )
                    } finally {
                        submitting = false
                    }
                }
            }
        }
    }
}

@Composable
private fun BackendChoiceDialog(
    status: CoreBackendChoiceStatus,
    onDismiss: () -> Unit,
    onConfirm: (CoreBackendChoice) -> Unit,
) {
    var selected by remember(status.revision) { mutableStateOf(status.desired) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.backend_choice_title)) },
        text = {
            Column {
                Text(stringResource(R.string.backend_choice_explanation))
                status.options.forEach { option ->
                    Row(Modifier.clickable { selected = option }) {
                        RadioButton(selected = selected == option, onClick = { selected = option })
                        Text(backendChoiceLabel(option))
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(selected) }, enabled = selected in status.options) { Text(stringResource(R.string.btn_save)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.btn_cancel)) } },
    )
}

@Composable
private fun backendChoiceLabel(choice: CoreBackendChoice): String =
    stringResource(
        when (choice) {
            CoreBackendChoice.Auto -> {
                R.string.backend_choice_auto
            }

            is CoreBackendChoice.Fixed -> {
                when (choice.backend) {
                    CoreNativeBackend.BUILTIN -> R.string.dashboard_backend_builtin
                    CoreNativeBackend.KMOD -> R.string.dashboard_backend_kmod
                    CoreNativeBackend.KPM -> R.string.dashboard_backend_kpm
                    CoreNativeBackend.ZYGISK -> R.string.dashboard_backend_zygisk
                }
            }
        },
    )

@Composable
private fun backendChoiceStageLabel(stage: CoreBackendChoiceStage): String =
    stringResource(
        when (stage) {
            CoreBackendChoiceStage.CURRENT -> R.string.backend_choice_current
            CoreBackendChoiceStage.PENDING_RESTART -> R.string.backend_choice_pending
            CoreBackendChoiceStage.UNVERIFIED -> R.string.backend_choice_unverified
        },
    )
