package dev.okhsunrog.vpnhide.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.CanonicalActivation
import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.NativeBackendId
import dev.okhsunrog.vpnhide.OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.StatusBanner
import dev.okhsunrog.vpnhide.StatusColors
import dev.okhsunrog.vpnhide.picker.TargetsCache
import dev.okhsunrog.vpnhide.ui.components.PreferenceRowSwitch
import dev.okhsunrog.vpnhide.ui.components.SectionHeader
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
internal fun FilesystemHidingSettingsSection() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val targets by TargetsCache.snapshot.collectAsState()
    val rootSnapshot by RootSnapshotCache.snapshot.collectAsState()
    var saving by remember { mutableStateOf(false) }
    var confirmationOpen by remember { mutableStateOf(false) }
    var saveStatus by remember { mutableStateOf<String?>(null) }
    val enabledFeatures =
        targets
            ?.canonicalConfig
            ?.settings
            ?.optionalFeatures
            .orEmpty()
    val enabled = OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS in enabledFeatures
    val runtimeState =
        remember(enabled, rootSnapshot) {
            resolveFilesystemHidingState(
                desiredEnabled = enabled,
                sections = rootSnapshot?.sections.orEmpty(),
            )
        }
    val savedMessage =
        stringResource(
            if (runtimeState.backend == NativeBackendId.Zygisk) {
                R.string.settings_filesystem_hiding_saved_zygisk
            } else {
                R.string.settings_filesystem_hiding_saved
            },
        )
    val failedMessage = stringResource(R.string.settings_filesystem_hiding_failed)

    fun persist(value: Boolean) {
        saving = true
        saveStatus = null
        scope.launch {
            val exit = withContext(Dispatchers.IO) { writeFilesystemHidingSetting(value) }
            saving = false
            saveStatus = if (exit == 0) savedMessage else failedMessage
            if (exit == 0) TargetsCache.refreshAfterSave(scope, context)
        }
    }

    LaunchedEffect(Unit) { TargetsCache.ensureLoaded(scope, context) }

    if (confirmationOpen) {
        AlertDialog(
            onDismissRequest = { if (!saving) confirmationOpen = false },
            title = { Text(stringResource(R.string.settings_filesystem_hiding_confirm_title)) },
            text = {
                Text(
                    stringResource(
                        if (runtimeState.backend == NativeBackendId.Zygisk) {
                            R.string.settings_filesystem_hiding_confirm_body_zygisk
                        } else {
                            R.string.settings_filesystem_hiding_confirm_body
                        },
                    ),
                )
            },
            confirmButton = {
                TextButton(
                    enabled = !saving,
                    onClick = {
                        confirmationOpen = false
                        persist(true)
                    },
                ) {
                    Text(stringResource(R.string.settings_filesystem_hiding_enable))
                }
            },
            dismissButton = {
                TextButton(
                    enabled = !saving,
                    onClick = { confirmationOpen = false },
                ) {
                    Text(stringResource(R.string.btn_cancel))
                }
            },
        )
    }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SectionHeader(
            text = stringResource(R.string.settings_experimental_protection),
            bold = false,
            modifier = Modifier.padding(start = 4.dp, top = 4.dp, bottom = 4.dp),
        )
        PreferenceRowSwitch(
            title = stringResource(R.string.settings_filesystem_hiding),
            subtitle = filesystemHidingStatusText(runtimeState),
            icon = Icons.Default.VisibilityOff,
            checked = enabled,
            enabled = targets != null && !saving && (runtimeState.nativeBackendInstalled || enabled),
            onCheckedChange = { value ->
                if (value) confirmationOpen = true else persist(false)
            },
        )
        StatusBanner(
            text = stringResource(R.string.settings_filesystem_hiding_sub),
            containerColor = StatusColors.warningContainer(),
            contentColor = MaterialTheme.colorScheme.onSurface,
        )
        saveStatus?.let { status ->
            Text(
                text = status,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(start = 4.dp),
            )
        }
    }
}

@Composable
private fun filesystemHidingStatusText(state: FilesystemHidingState): String =
    when (state.status) {
        FilesystemHidingStatus.Unavailable -> {
            stringResource(R.string.settings_filesystem_hiding_unavailable)
        }

        FilesystemHidingStatus.Disabled -> {
            stringResource(R.string.settings_filesystem_hiding_disabled)
        }

        FilesystemHidingStatus.Active -> {
            stringResource(R.string.settings_filesystem_hiding_active)
        }

        FilesystemHidingStatus.PendingEnable -> {
            stringResource(
                if (state.backend == NativeBackendId.Zygisk) {
                    R.string.settings_filesystem_hiding_pending_enable_zygisk
                } else {
                    R.string.settings_filesystem_hiding_pending_enable
                },
            )
        }

        FilesystemHidingStatus.PendingDisable -> {
            stringResource(
                if (state.backend == NativeBackendId.Zygisk) {
                    R.string.settings_filesystem_hiding_pending_disable_zygisk
                } else {
                    R.string.settings_filesystem_hiding_pending_disable
                },
            )
        }

        FilesystemHidingStatus.BootConfigError -> {
            stringResource(
                R.string.settings_filesystem_hiding_boot_error,
                state.errorDetail.orEmpty(),
            )
        }

        FilesystemHidingStatus.HookSetupError -> {
            stringResource(R.string.settings_filesystem_hiding_setup_error)
        }
    }

private suspend fun writeFilesystemHidingSetting(enabled: Boolean): Int {
    val snapshot = TargetsCache.snapshot.value ?: return 1
    // The config as stored, not a rebuild from the snapshot's per-role sets —
    // flipping an optional feature must leave the app list byte-identical.
    val base = snapshot.canonicalConfig ?: CanonicalConfig()
    val canonical =
        base.copy(
            settings =
                base.settings.copy(
                    optionalFeatures =
                        if (enabled) {
                            base.settings.optionalFeatures + OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS
                        } else {
                            base.settings.optionalFeatures - OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS
                        },
                ),
        )
    return CanonicalConfigRepository
        .commit(canonical, activation = CanonicalActivation(native = false))
        .exitCode
}
