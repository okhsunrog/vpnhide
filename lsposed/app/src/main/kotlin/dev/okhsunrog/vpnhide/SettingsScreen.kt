package dev.okhsunrog.vpnhide

import android.Manifest
import android.content.ClipData
import android.content.ClipboardManager
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Animation
import androidx.compose.material.icons.filled.BrightnessMedium
import androidx.compose.material.icons.filled.BugReport
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.DarkMode
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.DeleteForever
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.FileUpload
import androidx.compose.material.icons.filled.Forum
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Palette
import androidx.compose.material.icons.filled.RoundedCorner
import androidx.compose.material.icons.filled.Save
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.TextFields
import androidx.compose.material.icons.filled.Update
import androidx.compose.material.icons.filled.Vibration
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material.icons.filled.VpnKey
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.settings.CornerStyle
import dev.okhsunrog.vpnhide.settings.LocalSettingsInteractor
import dev.okhsunrog.vpnhide.settings.LocalSettingsState
import dev.okhsunrog.vpnhide.settings.ThemeMode
import dev.okhsunrog.vpnhide.ui.components.ButtonSpinner
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedOutlinedButton
import dev.okhsunrog.vpnhide.ui.components.PreferenceRow
import dev.okhsunrog.vpnhide.ui.components.PreferenceRowSwitch
import dev.okhsunrog.vpnhide.ui.components.SectionHeader
import dev.okhsunrog.vpnhide.ui.theme.AppColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.nio.charset.StandardCharsets

/** The small status line shown under a settings action (import/export result,
 * superkey save outcome). Renders nothing when [status] is null. */
@Composable
private fun SettingsStatusLine(status: String?) {
    status?.let {
        Text(
            text = it,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 4.dp),
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    selfNeedsRestart: Boolean?,
    onBack: () -> Unit,
) {
    val settings = LocalSettingsState.current
    val interactor = LocalSettingsInteractor.current
    var diagnosticsOpen by remember { mutableStateOf(false) }
    var hiddenAppsOpen by remember { mutableStateOf(false) }

    if (diagnosticsOpen) {
        DiagnosticsSettingsScreen(
            selfNeedsRestart = selfNeedsRestart,
            onBack = { diagnosticsOpen = false },
        )
        return
    }
    if (hiddenAppsOpen) {
        HiddenAppsSettingsScreen(onBack = { hiddenAppsOpen = false })
        return
    }

    Scaffold(
        containerColor = AppColors.screenBackground,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.settings_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = stringResource(R.string.action_back),
                        )
                    }
                },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = AppColors.topBarContainer,
                        titleContentColor = MaterialTheme.colorScheme.onSurface,
                        navigationIconContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                    ),
            )
        },
    ) { padding ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .verticalScroll(rememberScrollState())
                    .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            // ── Appearance ── one grouped block of four rows.
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                SettingsSectionHeader(stringResource(R.string.settings_appearance))

                val themeModeLabel =
                    when (settings.themeMode) {
                        ThemeMode.System -> stringResource(R.string.theme_mode_system)
                        ThemeMode.Light -> stringResource(R.string.theme_mode_light)
                        ThemeMode.Dark -> stringResource(R.string.theme_mode_dark)
                    }
                PreferenceRow(
                    title = stringResource(R.string.settings_theme_mode),
                    subtitle = themeModeLabel,
                    icon = Icons.Default.BrightnessMedium,
                    index = 0,
                    count = 4,
                    onClick = {
                        val next =
                            when (settings.themeMode) {
                                ThemeMode.System -> ThemeMode.Light
                                ThemeMode.Light -> ThemeMode.Dark
                                ThemeMode.Dark -> ThemeMode.System
                            }
                        interactor.setThemeMode(next)
                    },
                )
                PreferenceRowSwitch(
                    title = stringResource(R.string.settings_dynamic_color),
                    subtitle = stringResource(R.string.settings_dynamic_color_sub),
                    icon = Icons.Default.Palette,
                    index = 1,
                    count = 4,
                    checked = settings.dynamicColor,
                    onCheckedChange = interactor::setDynamicColor,
                )
                PreferenceRowSwitch(
                    title = stringResource(R.string.settings_amoled),
                    subtitle = stringResource(R.string.settings_amoled_sub),
                    icon = Icons.Default.DarkMode,
                    index = 2,
                    count = 4,
                    checked = settings.amoled,
                    onCheckedChange = interactor::setAmoled,
                )
                PreferenceRowSwitch(
                    title = stringResource(R.string.settings_squircle),
                    subtitle = stringResource(R.string.settings_squircle_sub),
                    icon = Icons.Default.RoundedCorner,
                    index = 3,
                    count = 4,
                    checked = settings.cornerStyle == CornerStyle.Smooth,
                    onCheckedChange = { value ->
                        interactor.setCornerStyle(if (value) CornerStyle.Smooth else CornerStyle.Rounded)
                    },
                )
            }

            // ── Interaction ── grouped block of three rows.
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                SettingsSectionHeader(stringResource(R.string.settings_interaction))
                PreferenceRowSwitch(
                    title = stringResource(R.string.settings_haptics),
                    subtitle = stringResource(R.string.settings_haptics_sub),
                    icon = Icons.Default.Vibration,
                    index = 0,
                    count = 3,
                    checked = settings.hapticsEnabled,
                    onCheckedChange = interactor::setHapticsEnabled,
                )
                PreferenceRowSwitch(
                    title = stringResource(R.string.settings_animations),
                    subtitle = stringResource(R.string.settings_animations_sub),
                    icon = Icons.Default.Animation,
                    index = 1,
                    count = 3,
                    checked = settings.animationsEnabled,
                    onCheckedChange = interactor::setAnimationsEnabled,
                )
                PreferenceRowSwitch(
                    title = stringResource(R.string.settings_full_role_labels),
                    subtitle = stringResource(R.string.settings_full_role_labels_sub),
                    icon = Icons.Default.TextFields,
                    index = 2,
                    count = 3,
                    checked = settings.fullProtectionRoleLabels,
                    onCheckedChange = interactor::setFullProtectionRoleLabels,
                )
            }

            UpdatesSettingsSection()
            AutoHideSettingsSection(onOpenHiddenApps = { hiddenAppsOpen = true })
            FilesystemHidingSettingsSection()
            DiagnosticsSettingsSection(onOpen = { diagnosticsOpen = true })
            DebugToolsSettingsSection(selfNeedsRestart = selfNeedsRestart)
            ConfigBackupSection()
            SuperkeySettingsSection()
            CommunitySettingsSection()
            ResetSettingsSection(selfNeedsRestart = selfNeedsRestart)
            DeveloperSettingsSection()
        }
    }
}

@Composable
private fun UpdatesSettingsSection() {
    val settings = LocalSettingsState.current
    val interactor = LocalSettingsInteractor.current
    val context = LocalContext.current
    val notificationPermissionLauncher =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
            // The switch reflects the user's preference. If Android denies the
            // notification permission, the worker still runs and skips delivery.
        }
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        SettingsSectionHeader(stringResource(R.string.settings_updates_section))
        PreferenceRowSwitch(
            title = stringResource(R.string.settings_background_update_checks),
            subtitle = stringResource(R.string.settings_background_update_checks_sub),
            icon = Icons.Default.Update,
            checked = settings.backgroundUpdateChecksEnabled,
            onCheckedChange = { enabled ->
                interactor.setBackgroundUpdateChecksEnabled(enabled)
                if (enabled && shouldRequestUpdateNotificationPermission(context)) {
                    notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
                }
            },
        )
    }
}

@Composable
private fun ResetSettingsSection(selfNeedsRestart: Boolean?) {
    var open by remember { mutableStateOf(false) }
    if (open) {
        FullResetDialog(selfNeedsRestart = selfNeedsRestart == true, onDismiss = { open = false })
    }
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        SettingsSectionHeader(stringResource(R.string.settings_reset_section))
        PreferenceRow(
            title = stringResource(R.string.reset_button),
            subtitle = stringResource(R.string.reset_button_sub),
            icon = Icons.Default.DeleteForever,
            onClick = { open = true },
        )
    }
}

@Composable
private fun CommunitySettingsSection() {
    var showContact by remember { mutableStateOf(false) }
    var showDonate by remember { mutableStateOf(false) }
    if (showContact) {
        ContactModal(onDismiss = { showContact = false })
    }
    if (showDonate) {
        DonateModal(onDismiss = { showDonate = false })
    }
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        SettingsSectionHeader(stringResource(R.string.settings_community_section))
        PreferenceRow(
            title = stringResource(R.string.settings_community),
            subtitle = stringResource(R.string.settings_community_sub),
            icon = Icons.Default.Forum,
            index = 0,
            count = 2,
            onClick = { showContact = true },
        )
        PreferenceRow(
            title = stringResource(R.string.settings_donate),
            subtitle = stringResource(R.string.settings_donate_sub),
            icon = Icons.Default.Favorite,
            index = 1,
            count = 2,
            onClick = { showDonate = true },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun DiagnosticsSettingsScreen(
    selfNeedsRestart: Boolean?,
    onBack: () -> Unit,
) {
    BackHandler(onBack = onBack)
    Scaffold(
        containerColor = AppColors.screenBackground,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.settings_diagnostics_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = stringResource(R.string.action_back),
                        )
                    }
                },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = AppColors.topBarContainer,
                        titleContentColor = MaterialTheme.colorScheme.onSurface,
                        navigationIconContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                    ),
            )
        },
    ) { padding ->
        if (selfNeedsRestart == null) {
            Box(
                modifier = Modifier.fillMaxSize().padding(padding),
                contentAlignment = Alignment.Center,
            ) {
                CircularProgressIndicator()
            }
        } else {
            DiagnosticsScreen(
                selfNeedsRestart = selfNeedsRestart,
                modifier = Modifier.padding(padding),
            )
        }
    }
}

@Composable
private fun DebugToolsSettingsSection(selfNeedsRestart: Boolean?) {
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        SettingsSectionHeader(stringResource(R.string.settings_debug_section))
        DebugToolsSection(selfNeedsRestart = selfNeedsRestart)
    }
}

@Composable
private fun DeveloperSettingsSection() {
    val settings = LocalSettingsState.current
    val interactor = LocalSettingsInteractor.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val debugLoggingByCanonicalConfig by TargetsCache.snapshot.collectAsState()
    val debugLogging =
        debugLoggingByCanonicalConfig?.canonicalConfig?.debugSwitch
            ?: VpnHideLog.enabled
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        SettingsSectionHeader(stringResource(R.string.settings_developer_section))
        PreferenceRowSwitch(
            title = stringResource(R.string.settings_suppress_version_warnings),
            subtitle = stringResource(R.string.settings_suppress_version_warnings_sub),
            icon = Icons.Default.Update,
            index = 0,
            count = 3,
            checked = settings.suppressVersionWarnings,
            onCheckedChange = interactor::setSuppressVersionWarnings,
        )
        // Off by default. The bridge ships in release too (the user develops on
        // release builds) — when on it opens a loopback control port, which the
        // dashboard surfaces as an info note so it isn't left running unnoticed.
        PreferenceRowSwitch(
            title = stringResource(R.string.settings_agent_control),
            subtitle = stringResource(R.string.settings_agent_control_sub),
            icon = Icons.Default.Settings,
            index = 1,
            count = 3,
            checked = settings.agentControlEnabled,
            onCheckedChange = interactor::setAgentControlEnabled,
        )
        PreferenceRowSwitch(
            title = stringResource(R.string.diag_debug_logging_title),
            subtitle = stringResource(R.string.settings_debug_logging_sub),
            icon = Icons.Default.BugReport,
            index = 2,
            count = 3,
            checked = debugLogging,
            onCheckedChange = { value ->
                scope.launch(Dispatchers.IO) { setDebugLoggingEnabled(value) }
            },
        )
    }
}

@Composable
private fun DiagnosticsSettingsSection(onOpen: () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        SettingsSectionHeader(stringResource(R.string.settings_diagnostics_section))
        PreferenceRow(
            title = stringResource(R.string.settings_diagnostics_title),
            subtitle = stringResource(R.string.settings_diagnostics_sub),
            icon = Icons.Default.CheckCircle,
            onClick = onOpen,
        )
    }
}

@Composable
private fun ConfigBackupSection() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val targets by TargetsCache.snapshot.collectAsState()
    var operation by remember { mutableStateOf(ConfigOperation.Idle) }
    var pendingExport by remember { mutableStateOf<String?>(null) }
    var pendingPackageListExport by remember { mutableStateOf<String?>(null) }
    var packageListDialogOpen by remember { mutableStateOf(false) }
    var legacyImportDialog by remember { mutableStateOf<LegacyImportPrompt?>(null) }
    var status by remember { mutableStateOf<String?>(null) }
    val rootSnapshot by RootSnapshotCache.snapshot.collectAsState()
    // Pre-1.0 config still on disk (see LegacyConfigImport). Derived from the
    // shared root snapshot TargetsCache already loads — no extra su round-trip.
    val legacyPrompt =
        remember(rootSnapshot, targets) {
            val uidToPkg = targets?.uidToPkg ?: return@remember null
            rootSnapshot?.let { parseLegacyConfigCandidate(it.sections, uidToPkg)?.toPrompt() }
        }
    val exportDone = stringResource(R.string.settings_config_export_done)
    val exportFailed = stringResource(R.string.settings_config_export_failed)
    val importDone = stringResource(R.string.settings_config_import_done)
    val importInvalid = stringResource(R.string.settings_config_import_invalid)
    val importRootFailed = stringResource(R.string.settings_config_import_root_failed)
    val packageListCopied = stringResource(R.string.settings_package_list_copied)
    val packageListSaved = stringResource(R.string.settings_package_list_saved)
    val packageListSaveFailed = stringResource(R.string.settings_package_list_save_failed)

    val exportLauncher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.CreateDocument("application/json"),
        ) { uri ->
            val raw = pendingExport
            pendingExport = null
            if (uri == null || raw == null) return@rememberLauncherForActivityResult
            operation = ConfigOperation.Export
            scope.launch {
                val ok = withContext(Dispatchers.IO) { writeTextToUri(context, uri, raw) }
                operation = ConfigOperation.Idle
                status = if (ok) exportDone else exportFailed
            }
        }

    val importLauncher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.OpenDocument(),
        ) { uri ->
            if (uri == null) return@rememberLauncherForActivityResult
            operation = ConfigOperation.Import
            status = null
            scope.launch {
                val result = withContext(Dispatchers.IO) { importConfigFromUri(context, uri) }
                operation = ConfigOperation.Idle
                status =
                    when (result) {
                        ConfigImportResult.Success -> {
                            TargetsCache.refreshAfterSave(scope, context)
                            importDone
                        }

                        ConfigImportResult.InvalidJson -> {
                            importInvalid
                        }

                        ConfigImportResult.RootFailed -> {
                            importRootFailed
                        }
                    }
            }
        }

    val packageListSaveLauncher =
        rememberLauncherForActivityResult(
            ActivityResultContracts.CreateDocument("text/plain"),
        ) { uri ->
            val raw = pendingPackageListExport
            pendingPackageListExport = null
            if (uri == null || raw == null) return@rememberLauncherForActivityResult
            operation = ConfigOperation.PackageListSave
            scope.launch {
                val ok = withContext(Dispatchers.IO) { writeTextToUri(context, uri, raw) }
                operation = ConfigOperation.Idle
                status = if (ok) packageListSaved else packageListSaveFailed
            }
        }

    LaunchedEffect(Unit) {
        TargetsCache.ensureLoaded(scope, context)
    }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SettingsSectionHeader(stringResource(R.string.settings_config_section))
        PreferenceRow(
            title = stringResource(R.string.settings_config_backup_title),
            subtitle = stringResource(R.string.settings_config_backup_sub),
            icon = Icons.Default.FileDownload,
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            EnhancedOutlinedButton(
                onClick = {
                    pendingExport = buildConfigExportJson(targets)
                    status = null
                    exportLauncher.launch("vpnhide_config.json")
                },
                enabled = operation == ConfigOperation.Idle && targets != null,
                modifier = Modifier.weight(1f),
            ) {
                if (operation == ConfigOperation.Export) {
                    ButtonSpinner(Modifier.padding(end = 8.dp))
                } else {
                    Icon(Icons.Default.FileDownload, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                }
                Text(stringResource(R.string.settings_config_export))
            }
            EnhancedButton(
                onClick = {
                    importLauncher.launch(
                        arrayOf(
                            "application/json",
                            "text/json",
                            "text/plain",
                            "application/octet-stream",
                            "*/*",
                        ),
                    )
                },
                enabled = operation == ConfigOperation.Idle,
                modifier = Modifier.weight(1f),
            ) {
                if (operation == ConfigOperation.Import) {
                    ButtonSpinner(Modifier.padding(end = 8.dp))
                } else {
                    Icon(Icons.Default.FileUpload, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                }
                Text(stringResource(R.string.settings_config_import))
            }
        }
        Text(
            text = stringResource(R.string.settings_config_import_note),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 4.dp),
        )
        PreferenceRow(
            title = stringResource(R.string.settings_package_list_title),
            subtitle = stringResource(R.string.settings_package_list_sub),
            icon = Icons.Default.ContentCopy,
            enabled = targets != null && operation == ConfigOperation.Idle,
            onClick = { packageListDialogOpen = true },
        )
        // Only rendered while pre-1.0 files are actually on disk — the import
        // deletes them, so this row disappears once it has run. Keeps the option
        // reachable for anyone who hid the Dashboard banner.
        legacyPrompt?.let { prompt ->
            PreferenceRow(
                title = stringResource(R.string.settings_legacy_import_title),
                subtitle = stringResource(R.string.settings_legacy_import_sub, prompt.packages),
                icon = Icons.Default.FileUpload,
                enabled = operation == ConfigOperation.Idle,
                onClick = { legacyImportDialog = prompt },
            )
        }
        SettingsStatusLine(status)
    }

    // Captured on tap: the import clears the on-disk files, so reading the
    // prompt live would tear the dialog down before it shows its result.
    legacyImportDialog?.let { prompt ->
        LegacyImportDialog(
            prompt = prompt,
            onDismiss = { legacyImportDialog = null },
        )
    }

    val packageListConfig = targets?.let(::buildConfigExportCanonical)
    if (packageListDialogOpen && packageListConfig != null) {
        PackageListExportDialog(
            config = packageListConfig,
            selfPkg = context.packageName,
            onDismiss = { packageListDialogOpen = false },
            onCopy = { text ->
                copyTextToClipboard(context, text)
                status = packageListCopied
                packageListDialogOpen = false
            },
            onSave = { text, source ->
                pendingPackageListExport = text
                status = null
                packageListDialogOpen = false
                packageListSaveLauncher.launch(packageListExportFileName(source))
            },
        )
    }
}

private fun copyTextToClipboard(
    context: android.content.Context,
    text: String,
) {
    val clipboard = context.getSystemService(ClipboardManager::class.java)
    clipboard.setPrimaryClip(ClipData.newPlainText("VPN Hide package list", text))
}

@Composable
private fun PackageListExportDialog(
    config: CanonicalConfig,
    selfPkg: String,
    onDismiss: () -> Unit,
    onCopy: (String) -> Unit,
    onSave: (String, PackageListSource) -> Unit,
) {
    var source by remember { mutableStateOf(PackageListSource.Java) }
    var format by remember { mutableStateOf(PackageListFormat.Comma) }
    val packages = remember(config, source, selfPkg) { packageListExportPackages(config, source, selfPkg) }
    val text = remember(config, source, format, selfPkg) { formatPackageListExport(config, source, format, selfPkg) }
    val hasPackages = packages.isNotEmpty()

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.settings_package_list_title)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    text = stringResource(R.string.settings_package_list_dialog_body),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    text = stringResource(R.string.settings_package_list_source),
                    style = MaterialTheme.typography.titleSmall,
                )
                PackageListSource.entries.forEach { option ->
                    PackageListRadioRow(
                        label = packageListSourceLabel(option),
                        selected = source == option,
                        onClick = { source = option },
                    )
                }
                Text(
                    text = stringResource(R.string.settings_package_list_format),
                    style = MaterialTheme.typography.titleSmall,
                )
                PackageListFormat.entries.forEach { option ->
                    PackageListRadioRow(
                        label = packageListFormatLabel(option),
                        selected = format == option,
                        onClick = { format = option },
                    )
                }
                Text(
                    text =
                        if (hasPackages) {
                            stringResource(R.string.settings_package_list_count, packages.size)
                        } else {
                            stringResource(R.string.settings_package_list_empty)
                        },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(
                    onClick = { onSave(text, source) },
                    enabled = hasPackages,
                ) {
                    Icon(Icons.Default.Save, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text(stringResource(R.string.settings_package_list_save))
                }
                TextButton(
                    onClick = { onCopy(text) },
                    enabled = hasPackages,
                ) {
                    Icon(Icons.Default.ContentCopy, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text(stringResource(R.string.settings_package_list_copy))
                }
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text(stringResource(R.string.btn_cancel))
            }
        },
    )
}

@Composable
private fun PackageListRadioRow(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onClick)
                .padding(vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RadioButton(selected = selected, onClick = onClick)
        Spacer(Modifier.width(8.dp))
        Text(text = label, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun packageListSourceLabel(source: PackageListSource): String =
    when (source) {
        PackageListSource.Java -> stringResource(R.string.settings_package_list_source_java)
        PackageListSource.Native -> stringResource(R.string.settings_package_list_source_native)
        PackageListSource.AppHiding -> stringResource(R.string.settings_package_list_source_app_hiding)
        PackageListSource.Ports -> stringResource(R.string.settings_package_list_source_ports)
        PackageListSource.AllProtection -> stringResource(R.string.settings_package_list_source_all)
    }

@Composable
private fun packageListFormatLabel(format: PackageListFormat): String =
    when (format) {
        PackageListFormat.Comma -> stringResource(R.string.settings_package_list_format_comma)
        PackageListFormat.Lines -> stringResource(R.string.settings_package_list_format_lines)
    }

@Composable
private fun SuperkeySettingsSection() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val targets by TargetsCache.snapshot.collectAsState()
    var superkey by remember { mutableStateOf("") }
    var saving by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf<String?>(null) }
    val remembered = targets?.apatchSuperkeySaved == true
    val storedMessage = stringResource(R.string.settings_superkey_stored)
    val clearedMessage = stringResource(R.string.settings_superkey_cleared)
    val failedMessage = stringResource(R.string.settings_superkey_failed)

    LaunchedEffect(Unit) {
        TargetsCache.ensureLoaded(scope, context)
    }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        SettingsSectionHeader(stringResource(R.string.settings_security))
        PreferenceRow(
            title = stringResource(R.string.settings_superkey_title),
            subtitle =
                stringResource(
                    if (remembered) {
                        R.string.settings_superkey_saved
                    } else {
                        R.string.settings_superkey_not_saved
                    },
                ),
            icon = Icons.Default.Lock,
        )
        OutlinedTextField(
            value = superkey,
            onValueChange = {
                superkey = it
                status = null
            },
            label = { Text(stringResource(R.string.settings_superkey_placeholder)) },
            visualTransformation = PasswordVisualTransformation(),
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Row(modifier = Modifier.fillMaxWidth()) {
            TextButton(
                onClick = {
                    saving = true
                    status = null
                    scope.launch {
                        val exit = withContext(Dispatchers.IO) { writeSuperkeySetting(remember = false, superkey = "") }
                        saving = false
                        status = if (exit == 0) clearedMessage else failedMessage
                        if (exit == 0) {
                            superkey = ""
                            TargetsCache.refresh(scope, context)
                        }
                    }
                },
                enabled = !saving && targets != null,
            ) {
                Text(stringResource(R.string.settings_superkey_clear))
            }
            Spacer(Modifier.width(8.dp))
            EnhancedButton(
                onClick = {
                    saving = true
                    status = null
                    val keyToWrite = superkey
                    scope.launch {
                        val exit = withContext(Dispatchers.IO) { writeSuperkeySetting(remember = true, superkey = keyToWrite) }
                        saving = false
                        status = if (exit == 0) storedMessage else failedMessage
                        if (exit == 0) {
                            superkey = ""
                            TargetsCache.refresh(scope, context)
                        }
                    }
                },
                enabled = !saving && superkey.isNotBlank() && targets != null,
                modifier = Modifier.weight(1f),
            ) {
                if (saving) {
                    ButtonSpinner(Modifier.padding(end = 8.dp))
                }
                Text(stringResource(R.string.settings_superkey_store))
            }
        }
        SettingsStatusLine(status)
    }
}

private suspend fun writeSuperkeySetting(
    remember: Boolean,
    superkey: String,
): Int {
    // Read the config itself, never a rebuild from the snapshot's projections:
    // this toggles one settings field and must not rewrite the app list on its
    // way past. A missing config means there is nothing to preserve yet.
    val base = TargetsCache.snapshot.value?.canonicalConfig ?: CanonicalConfig()
    val canonical = base.copy(settings = base.settings.copy(rememberSuperkey = remember))
    val secretCommand = if (remember) buildSuperkeyWriteCommand(superkey) else buildSuperkeyClearCommand()
    return CanonicalConfigRepository
        .commit(
            canonical,
            coupledCommands = listOf(secretCommand),
            activation = CanonicalActivation(native = remember),
        ).exitCode
}

private enum class ConfigImportResult {
    Success,
    InvalidJson,
    RootFailed,
}

private enum class ConfigOperation {
    Idle,
    Export,
    Import,
    PackageListSave,
}

private fun buildConfigExportCanonical(snapshot: TargetsSnapshot?): CanonicalConfig =
    when {
        snapshot?.canonicalConfig != null -> snapshot.canonicalConfig
        snapshot != null -> buildCanonicalConfigFromTargetsSnapshot(snapshot)
        else -> CanonicalConfig()
    }

private fun buildConfigExportJson(snapshot: TargetsSnapshot?): String = canonicalConfigJson(buildConfigExportCanonical(snapshot))

private fun writeTextToUri(
    context: android.content.Context,
    uri: Uri,
    text: String,
): Boolean =
    runCatching {
        context.contentResolver.openOutputStream(uri)?.use { out ->
            out.write(text.toByteArray(StandardCharsets.UTF_8))
        } ?: error("openOutputStream returned null")
    }.isSuccess

private suspend fun importConfigFromUri(
    context: android.content.Context,
    uri: Uri,
): ConfigImportResult {
    val raw =
        runCatching {
            context.contentResolver
                .openInputStream(uri)
                ?.bufferedReader(StandardCharsets.UTF_8)
                ?.use { it.readText() }
        }.getOrNull() ?: return ConfigImportResult.InvalidJson
    val canonical = parseImportedCanonicalConfig(raw, context.packageName) ?: return ConfigImportResult.InvalidJson
    val result =
        CanonicalConfigRepository.commit(
            canonical,
            activation = CanonicalActivation(native = true, ports = true),
        )
    if (!result.succeeded) return ConfigImportResult.RootFailed
    VpnHideLog.enabled = canonical.debug
    return ConfigImportResult.Success
}

@Composable
private fun AutoHideSettingsSection(onOpenHiddenApps: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val targets by TargetsCache.snapshot.collectAsState()
    val apps by AppListCache.apps.collectAsState()
    var saving by remember { mutableStateOf<AutoHideSetting?>(null) }
    var status by remember { mutableStateOf<String?>(null) }
    var unavailableDialogOpen by remember { mutableStateOf(false) }
    val savedMessage = stringResource(R.string.settings_auto_hide_saved)
    val failedMessage = stringResource(R.string.settings_auto_hide_failed)
    val unavailableRemovedMessage = stringResource(R.string.settings_unavailable_configured_removed)
    val unavailableFailedMessage = stringResource(R.string.settings_unavailable_configured_failed)
    val canonical = targets?.let(::buildCanonicalConfigFromTargetsSnapshot)
    val settings = canonical?.settings ?: CanonicalSettings()
    val hiddenSummary =
        remember(canonical, apps, context.packageName) {
            val cfg = canonical
            val appList = apps
            if (cfg == null || appList == null) {
                null
            } else {
                hiddenAppsSummary(
                    hiddenAppStates(
                        config = cfg,
                        selfPkg = context.packageName,
                        signals = appList.map(AppSummary::toAutoHideSignal),
                    ),
                )
            }
        }
    val unavailableConfigured =
        remember(canonical, apps, context.packageName) {
            val cfg = canonical
            val appList = apps
            if (cfg == null || appList == null) {
                emptyList()
            } else {
                unavailableConfiguredApps(
                    config = cfg,
                    visiblePackages = appList.mapTo(mutableSetOf()) { it.packageName },
                    selfPkg = context.packageName,
                )
            }
        }
    val canWrite = targets != null && apps != null && saving == null

    fun updateSetting(
        setting: AutoHideSetting,
        transform: (CanonicalSettings) -> CanonicalSettings,
    ) {
        saving = setting
        status = null
        val appSignals = apps.orEmpty()
        scope.launch {
            val exit = withContext(Dispatchers.IO) { writeAutoHideSetting(context, appSignals, transform) }
            saving = null
            status = if (exit == 0) savedMessage else failedMessage
            if (exit == 0) {
                TargetsCache.refreshAfterSave(scope, context)
            }
        }
    }

    fun removeUnavailableConfigured(packages: Set<String>) {
        saving = AutoHideSetting.UnavailableConfigured
        status = null
        scope.launch {
            val exit = withContext(Dispatchers.IO) { writeRemoveUnavailableConfiguredApps(context, packages) }
            saving = null
            status = if (exit == 0) unavailableRemovedMessage else unavailableFailedMessage
            if (exit == 0) {
                unavailableDialogOpen = false
                TargetsCache.refreshAfterSave(scope, context)
            }
        }
    }

    LaunchedEffect(Unit) {
        TargetsCache.ensureLoaded(scope, context)
        AppListCache.ensureLoaded(scope, context)
    }

    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        SettingsSectionHeader(stringResource(R.string.settings_advanced_protection))
        PreferenceRowSwitch(
            title = stringResource(R.string.settings_auto_hide_vpn_services),
            subtitle = stringResource(R.string.settings_auto_hide_vpn_services_sub),
            icon = Icons.Default.VpnKey,
            index = 0,
            count = 4,
            checked = settings.autoHideVpnServices,
            enabled = canWrite,
            onCheckedChange = { enabled ->
                updateSetting(AutoHideSetting.VpnService) { it.copy(autoHideVpnServices = enabled) }
            },
        )
        PreferenceRowSwitch(
            title = stringResource(R.string.settings_auto_hide_vpn_name),
            subtitle = stringResource(R.string.settings_auto_hide_vpn_name_sub),
            icon = Icons.Default.TextFields,
            index = 1,
            count = 4,
            checked = settings.autoHideVpnName,
            enabled = canWrite,
            onCheckedChange = { enabled ->
                updateSetting(AutoHideSetting.VpnName) { it.copy(autoHideVpnName = enabled) }
            },
        )
        PreferenceRow(
            title = stringResource(R.string.settings_hidden_apps),
            subtitle =
                hiddenSummary?.let {
                    stringResource(R.string.settings_hidden_apps_sub, it.hidden, it.automatic, it.manual)
                } ?: stringResource(R.string.settings_unavailable_configured_loading),
            icon = Icons.Default.VisibilityOff,
            index = 2,
            count = 4,
            enabled = canWrite,
            onClick = onOpenHiddenApps,
        )
        PreferenceRow(
            title = stringResource(R.string.settings_unavailable_configured_title),
            subtitle =
                if (targets == null || apps == null) {
                    stringResource(R.string.settings_unavailable_configured_loading)
                } else {
                    stringResource(R.string.settings_unavailable_configured_sub, unavailableConfigured.size)
                },
            icon = Icons.Default.Delete,
            index = 3,
            count = 4,
            enabled = canWrite && unavailableConfigured.isNotEmpty(),
            onClick = { unavailableDialogOpen = true },
        )
        SettingsStatusLine(status)
    }

    if (unavailableDialogOpen) {
        UnavailableConfiguredAppsDialog(
            apps = unavailableConfigured,
            saving = saving == AutoHideSetting.UnavailableConfigured,
            onDismiss = { if (saving != AutoHideSetting.UnavailableConfigured) unavailableDialogOpen = false },
            onRemove = { app -> removeUnavailableConfigured(setOf(app.packageName)) },
            onRemoveAll = { removeUnavailableConfigured(unavailableConfigured.mapTo(mutableSetOf()) { it.packageName }) },
        )
    }
}

private enum class AutoHideSetting {
    VpnService,
    VpnName,
    UnavailableConfigured,
}

@Composable
private fun UnavailableConfiguredAppsDialog(
    apps: List<UnavailableConfiguredApp>,
    saving: Boolean,
    onDismiss: () -> Unit,
    onRemove: (UnavailableConfiguredApp) -> Unit,
    onRemoveAll: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.settings_unavailable_configured_title)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    text = stringResource(R.string.settings_unavailable_configured_body),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                LazyColumn(modifier = Modifier.heightIn(max = 420.dp)) {
                    items(apps, key = { it.packageName }) { app ->
                        UnavailableConfiguredAppRow(
                            app = app,
                            saving = saving,
                            onRemove = { onRemove(app) },
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onRemoveAll, enabled = !saving && apps.isNotEmpty()) {
                Icon(Icons.Default.Delete, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text(stringResource(R.string.settings_unavailable_configured_remove_all))
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, enabled = !saving) {
                Text(stringResource(R.string.btn_cancel))
            }
        },
    )
}

@Composable
private fun UnavailableConfiguredAppRow(
    app: UnavailableConfiguredApp,
    saving: Boolean,
    onRemove: () -> Unit,
) {
    val roleLabels =
        mapOf(
            ConfiguredAppRole.Java to stringResource(R.string.chip_java_full),
            ConfiguredAppRole.Native to stringResource(R.string.chip_native_full),
            ConfiguredAppRole.AppHiding to stringResource(R.string.chip_app_hiding_full),
            ConfiguredAppRole.Ports to stringResource(R.string.chip_ports_full),
            ConfiguredAppRole.Hidden to stringResource(R.string.settings_role_hidden),
        )
    val roleText = app.roles.joinToString(", ") { roleLabels.getValue(it) }
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = app.packageName,
                style = MaterialTheme.typography.bodyMedium,
                fontFamily = FontFamily.Monospace,
            )
            Text(
                text = roleText,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        TextButton(onClick = onRemove, enabled = !saving) {
            Text(stringResource(R.string.settings_unavailable_configured_remove))
        }
    }
}

private suspend fun writeAutoHideSetting(
    context: android.content.Context,
    apps: List<AppSummary>,
    transform: (CanonicalSettings) -> CanonicalSettings,
): Int {
    val snapshot = TargetsCache.snapshot.value
    val base =
        snapshot?.let(::buildCanonicalConfigFromTargetsSnapshot)
            ?: CanonicalConfig()
    val canonical =
        applyAutoHiddenPackages(
            config = base.copy(settings = transform(base.settings)),
            selfPkg = context.packageName,
            signals = apps.map(AppSummary::toAutoHideSignal),
        )
    return CanonicalConfigRepository.commit(canonical).exitCode
}

private suspend fun writeRemoveUnavailableConfiguredApps(
    context: android.content.Context,
    packages: Set<String>,
): Int {
    if (packages.isEmpty()) return 0
    val snapshot = TargetsCache.snapshot.value ?: return 1
    val base = buildCanonicalConfigFromTargetsSnapshot(snapshot)
    val canonical = removeConfiguredPackages(base, packages, context.packageName)
    return CanonicalConfigRepository
        .commit(
            canonical,
            activation = CanonicalActivation(native = true, ports = true),
        ).exitCode
}

// Settings headers use the shared non-bold, indented variant.
@Composable
private fun SettingsSectionHeader(text: String) {
    SectionHeader(
        text = text,
        bold = false,
        modifier = Modifier.padding(start = 4.dp, top = 4.dp, bottom = 4.dp),
    )
}
