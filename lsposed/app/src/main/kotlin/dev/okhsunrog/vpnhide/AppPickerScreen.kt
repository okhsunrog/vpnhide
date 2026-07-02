package dev.okhsunrog.vpnhide

import android.graphics.drawable.Drawable
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Layers
import androidx.compose.material.icons.filled.TextFields
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material.icons.filled.VpnKey
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.settings.LocalSettingsState

/**
 * One row per app across every VPN-hiding role. The unified list keeps role
 * selection in one place, and one Save writes the complete hiding config. Roles:
 *
 *  - [java]      "J" — LSPosed (the Java layer)
 *  - [native]    "N" — the one active native backend (kmod / KPM / Zygisk, §1.5);
 *                       written to every installed native backend, only the
 *                       active one acts.
 *  - [appHiding] "A" — observer of the hidden-package view (which packages get
 *                       hidden is auto-detected; see autoDetectHiddenPackages).
 *  - [ports]     "P" — localhost-port blocking observer.
 */
internal data class AppEntry(
    override val packageName: String,
    override val label: String,
    override val icon: Drawable?,
    override val isSystem: Boolean,
    override val userIds: List<Int> = emptyList(),
    val java: Boolean = false,
    val javaHooks: List<String>? = null,
    val native: Boolean = false,
    val nativeOverrides: NativeHookOverrides = NativeHookOverrides(),
    val appHiding: Boolean = false,
    val ports: Boolean = false,
    val portPolicy: PortPolicy? = null,
    val declaresVpnService: Boolean = false,
    val nameContainsVpn: Boolean = false,
) : TargetEntry {
    override val anySelected get() = java || native || appHiding || ports
}

internal enum class Layer { JAVA, NATIVE, APP_HIDING, PORTS }

@Composable
internal fun AppPickerScreen(
    searchQuery: String,
    showSystem: Boolean,
    showRussianOnly: Boolean,
    sortMode: TargetListSortMode,
    modifier: Modifier = Modifier,
) {
    TargetPickerScreen(
        searchQuery = searchQuery,
        showSystem = showSystem,
        showRussianOnly = showRussianOnly,
        sortMode = sortMode,
        modifier = modifier,
        helpPrefKey = "apps_unified",
        helpTitle = stringResource(R.string.apps_help_title),
        help = { targets -> AppsHelpContent(targets) },
        merge = { apps, t, selfPkg ->
            val nativeTargets = t.nativeTargets
            val observers = t.observerNames
            val autoHideSignals = apps.map(AppSummary::toAutoHideSignal)
            val baseCanonical = t.canonicalConfig ?: buildCanonicalConfigFromTargetsSnapshot(t)
            val autoApplied =
                applyAutoHiddenPackages(
                    config = baseCanonical,
                    selfPkg = selfPkg,
                    signals = autoHideSignals,
                )
            MergeResult(
                apps
                    .filter { it.packageName != selfPkg }
                    .map { app ->
                        val canonicalApp = baseCanonical.apps[app.packageName]
                        val nativeRole = canonicalApp?.native
                        AppEntry(
                            packageName = app.packageName,
                            label = app.label,
                            icon = app.icon,
                            isSystem = app.isSystem,
                            userIds = app.userIds,
                            java = canonicalApp?.java ?: (app.packageName in t.lsposedTargets),
                            javaHooks = canonicalApp?.takeIf { it.java }?.javaHooks?.takeIf { it.isNotEmpty() },
                            native = app.packageName in nativeTargets,
                            nativeOverrides = nativeRole?.overrides ?: NativeHookOverrides(),
                            appHiding = app.packageName in observers,
                            ports = app.packageName in t.portsObservers,
                            portPolicy = canonicalApp?.takeIf { it.ports }?.portPolicy,
                            declaresVpnService = app.declaresVpnService,
                            nameContainsVpn = app.nameContainsVpn,
                        )
                    },
                resaveNeeded = autoApplied != baseCanonical,
            )
        },
        countText = { entries, res ->
            res.getString(R.string.selected_count, entries.count { it.anySelected })
        },
        buildSaveCommand = { entries, ctx ->
            buildUnifiedSaveCommand(
                ctx = ctx,
                selections = entries.map(AppEntry::toRoleSelection),
                autoHideSignals = entries.map(AppEntry::toAutoHideSignal),
            )
        },
        successMessage = { entries, res ->
            res.getString(R.string.save_success, entries.count { it.anySelected })
        },
    ) { app, userNames, targets, onChange ->
        val nativeHookFamily = targets.nativeHookFamily
        AppRow(
            app = app,
            userNames = userNames,
            anyNativeInstalled = targets.anyNativeInstalled,
            nativeBackendId = targets.displayNativeBackendId,
            nativeHookFamily = nativeHookFamily,
            portsInstalled = targets.portsModuleInstalled,
            onToggle = { layer ->
                onChange(
                    when (layer) {
                        Layer.JAVA -> app.copy(java = !app.java, javaHooks = null)
                        Layer.NATIVE -> app.copy(native = !app.native, nativeOverrides = NativeHookOverrides())
                        Layer.APP_HIDING -> app.copy(appHiding = !app.appHiding)
                        Layer.PORTS -> app.copy(ports = !app.ports)
                    },
                )
            },
            onJavaHooksChange = { hooks ->
                onChange(
                    app.copy(
                        java = hooks == null || hooks.isNotEmpty(),
                        javaHooks = hooks?.takeIf { it.isNotEmpty() },
                    ),
                )
            },
            onNativeHooksChange = { hooks ->
                onChange(
                    app.copy(
                        native = hooks == null || hooks.isNotEmpty(),
                        nativeOverrides =
                            if (hooks == null || hooks.isNotEmpty()) {
                                app.nativeOverrides.withHooksFor(
                                    nativeHookFamily,
                                    hooks?.takeIf { it.isNotEmpty() },
                                )
                            } else {
                                NativeHookOverrides()
                            },
                    ),
                )
            },
            onPortPolicyChange = { policy ->
                onChange(
                    app.copy(
                        ports = true,
                        portPolicy = policy,
                    ),
                )
            },
            onToggleAll = {
                val newState = !app.anySelected
                onChange(
                    app.copy(
                        java = newState,
                        javaHooks = null,
                        native = if (targets.anyNativeInstalled) newState else false,
                        nativeOverrides = NativeHookOverrides(),
                        appHiding = newState,
                        ports = if (targets.portsModuleInstalled) newState else false,
                    ),
                )
            },
        )
    }
}

@Composable
private fun AppsHelpContent(targets: TargetsSnapshot) {
    val fullRoleLabels = LocalSettingsState.current.fullProtectionRoleLabels
    val primary = MaterialTheme.colorScheme.primary
    val secondary = MaterialTheme.colorScheme.secondary
    val tertiary = MaterialTheme.colorScheme.tertiary
    val error = MaterialTheme.colorScheme.error

    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        HelpInfoBlock(
            title =
                roleHelpLabel(
                    compact = stringResource(R.string.chip_java),
                    full = stringResource(R.string.chip_java_full),
                    fullLabels = fullRoleLabels,
                ),
            body = stringResource(R.string.apps_help_role_java_body),
            icon = Icons.Default.TextFields,
            color = primary,
        )
        HelpInfoBlock(
            title =
                roleHelpLabel(
                    compact = stringResource(R.string.chip_native),
                    full = stringResource(R.string.chip_native_full),
                    fullLabels = fullRoleLabels,
                ),
            body = stringResource(R.string.apps_help_role_native_body),
            icon = Icons.Default.VpnKey,
            color = secondary,
        )
        HelpInfoBlock(
            title =
                roleHelpLabel(
                    compact = stringResource(R.string.chip_app_hiding),
                    full = stringResource(R.string.chip_app_hiding_full),
                    fullLabels = fullRoleLabels,
                ),
            body = stringResource(R.string.apps_help_role_apps_body),
            icon = Icons.Default.VisibilityOff,
            color = tertiary,
        )
        HelpInfoBlock(
            title =
                roleHelpLabel(
                    compact = stringResource(R.string.chip_ports),
                    full = stringResource(R.string.chip_ports_full),
                    fullLabels = fullRoleLabels,
                ),
            body = stringResource(R.string.apps_help_role_ports_body),
            icon = Icons.Default.Layers,
            color = primary,
        )
        HelpInfoBlock(
            title = stringResource(R.string.apps_help_hook_settings_title),
            body = stringResource(R.string.apps_help_hook_settings_body),
            icon = Icons.Default.Tune,
            color = secondary,
        )
        HelpInfoBlock(
            title = stringResource(R.string.apps_help_apps_hiding_title),
            body = stringResource(R.string.apps_help_apps_hiding_body),
            icon = Icons.Default.VisibilityOff,
            color = tertiary,
        )
        HelpInfoBlock(
            title = stringResource(R.string.apps_help_apply_title),
            body = stringResource(R.string.apps_help_apply_body),
            icon = Icons.Default.Layers,
            color = primary,
        )
        if (targets.activeNativeBackendId == NativeBackendId.Zygisk) {
            HelpInfoBlock(
                title = stringResource(R.string.apps_help_zygisk_warning_title),
                body = stringResource(R.string.apps_help_zygisk_warning_body),
                icon = Icons.Default.Warning,
                color = error,
            )
        }
    }
}

private fun roleHelpLabel(
    compact: String,
    full: String,
    fullLabels: Boolean,
): String = if (fullLabels) "$full ($compact)" else "$compact ($full)"

@Composable
private fun HelpInfoBlock(
    title: String,
    body: String,
    icon: ImageVector,
    color: Color,
) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(6.dp),
        color = color.copy(alpha = 0.08f),
        border = BorderStroke(1.dp, color.copy(alpha = 0.42f)),
    ) {
        Row(
            modifier = Modifier.padding(12.dp),
            verticalAlignment = Alignment.Top,
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = color,
                modifier = Modifier.size(18.dp),
            )
            Spacer(Modifier.width(10.dp))
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.onSurface,
                )
                Text(
                    text = body,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/**
 * Build the single root command that persists every role at once. The canonical
 * JSON is the single persistent source of truth. Native backends are updated by
 * running the installed activator; LSPosed reads the JSON directly; the ports
 * activator derives its observer set from the same JSON.
 */
private fun buildUnifiedSaveCommand(
    ctx: SaveContext,
    selections: Collection<AppRoleSelection>,
    autoHideSignals: Collection<AppAutoHideSignal>,
): String {
    val parts = mutableListOf<String>()

    val canonical =
        buildCanonicalConfigForAppPickerSave(
            debug = ctx.debug,
            selfPkg = ctx.selfPkg,
            selections = selections,
            snapshot = TargetsCache.snapshot.value,
            autoHideSignals = autoHideSignals,
        )
    parts += buildCanonicalConfigWriteCommand(canonical)

    parts += ConfigChannels.nativeWriteParts()

    parts += ConfigChannels.portsActivatorCommand()

    return parts.joinToString(" ; ")
}

private fun AppEntry.toRoleSelection(): AppRoleSelection =
    AppRoleSelection(
        packageName = packageName,
        java = java,
        javaHooks = javaHooks,
        native = native,
        nativeOverrides = nativeOverrides,
        appHiding = appHiding,
        ports = ports,
        portPolicy = portPolicy,
    )

private fun AppEntry.toAutoHideSignal(): AppAutoHideSignal =
    AppAutoHideSignal(
        packageName = packageName,
        declaresVpnService = declaresVpnService,
        nameContainsVpn = nameContainsVpn,
    )

@Composable
private fun AppRow(
    app: AppEntry,
    userNames: Map<Int, String>,
    anyNativeInstalled: Boolean,
    nativeBackendId: NativeBackendId?,
    nativeHookFamily: NativeHookFamily,
    portsInstalled: Boolean,
    onToggle: (Layer) -> Unit,
    onJavaHooksChange: (List<String>?) -> Unit,
    onNativeHooksChange: (List<String>?) -> Unit,
    onPortPolicyChange: (PortPolicy?) -> Unit,
    onToggleAll: () -> Unit,
) {
    var javaHookDialogOpen by remember { mutableStateOf(false) }
    var nativeHookDialogOpen by remember { mutableStateOf(false) }
    var portsDialogOpen by remember { mutableStateOf(false) }
    val fullRoleLabels = LocalSettingsState.current.fullProtectionRoleLabels
    val nativeHooks = app.nativeOverrides.hooksFor(nativeHookFamily)
    TargetRowShell(
        label = app.label,
        packageName = app.packageName,
        icon = app.icon,
        userIds = app.userIds,
        userNames = userNames,
        modifier = Modifier.clickable(onClick = onToggleAll),
    ) {
        HookTargetChip(
            label =
                roleLabel(
                    compact = stringResource(R.string.chip_java),
                    full = stringResource(R.string.chip_java_full),
                    partial = app.javaHooks != null,
                    fullLabels = fullRoleLabels,
                ),
            enabled = app.java,
            onToggle = { onToggle(Layer.JAVA) },
            onConfigure = { javaHookDialogOpen = true },
            contentDescription = stringResource(R.string.java_hooks_title),
        )
        if (anyNativeInstalled) {
            HookTargetChip(
                label =
                    roleLabel(
                        compact = stringResource(R.string.chip_native),
                        full = stringResource(R.string.chip_native_full),
                        partial = nativeHooks != null,
                        fullLabels = fullRoleLabels,
                    ),
                enabled = app.native,
                onToggle = { onToggle(Layer.NATIVE) },
                onConfigure = { nativeHookDialogOpen = true },
                contentDescription = stringResource(R.string.native_hooks_title),
            )
        }
        TargetChip(
            label =
                if (fullRoleLabels) {
                    stringResource(R.string.chip_app_hiding_full)
                } else {
                    stringResource(R.string.chip_app_hiding)
                },
            enabled = app.appHiding,
        ) {
            onToggle(Layer.APP_HIDING)
        }
        if (portsInstalled) {
            HookTargetChip(
                label =
                    roleLabel(
                        compact = stringResource(R.string.chip_ports),
                        full = stringResource(R.string.chip_ports_full),
                        partial = app.ports && app.portPolicy != null,
                        fullLabels = fullRoleLabels,
                    ),
                enabled = app.ports,
                onToggle = { onToggle(Layer.PORTS) },
                onConfigure = { portsDialogOpen = true },
                contentDescription = stringResource(R.string.ports_policy_title),
            )
        }
    }

    if (javaHookDialogOpen) {
        HooksDialog(
            app = app,
            title = stringResource(R.string.java_hooks_title),
            hookEntries = LsposedJavaHookEntries,
            selectedHooks = app.javaHooks,
            roleEnabled = app.java,
            onDismiss = { javaHookDialogOpen = false },
            onSave = { hooks ->
                onJavaHooksChange(hooks)
                javaHookDialogOpen = false
            },
        )
    }

    if (nativeHookDialogOpen) {
        HooksDialog(
            app = app,
            title = nativeHooksTitle(nativeBackendId),
            hookEntries = nativeHookEntriesFor(nativeHookFamily),
            selectedHooks = nativeHooks,
            roleEnabled = app.native,
            onDismiss = { nativeHookDialogOpen = false },
            onSave = { hooks ->
                onNativeHooksChange(hooks)
                nativeHookDialogOpen = false
            },
        )
    }

    if (portsDialogOpen) {
        PortsPolicyDialog(
            app = app,
            portPolicy = app.portPolicy,
            onDismiss = { portsDialogOpen = false },
            onSave = { policy ->
                onPortPolicyChange(policy)
                portsDialogOpen = false
            },
        )
    }
}

private fun roleLabel(
    compact: String,
    full: String,
    partial: Boolean,
    fullLabels: Boolean,
): String = (if (fullLabels) full else compact) + if (partial) "*" else ""

@Composable
private fun nativeHooksTitle(backend: NativeBackendId?): String {
    val backendName =
        when (backend) {
            NativeBackendId.Kmod -> stringResource(R.string.dashboard_backend_kmod)
            NativeBackendId.Kpm -> stringResource(R.string.dashboard_backend_kpm)
            NativeBackendId.Zygisk -> stringResource(R.string.dashboard_backend_zygisk)
            null -> stringResource(R.string.chip_native_full)
        }
    return stringResource(R.string.native_hooks_title_with_backend, backendName)
}

@Composable
private fun HookTargetChip(
    label: String,
    enabled: Boolean,
    onToggle: () -> Unit,
    onConfigure: () -> Unit,
    contentDescription: String,
) {
    val containerColor = if (enabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant
    val contentColor = if (enabled) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant
    Surface(
        shape = RoundedCornerShape(4.dp),
        color = containerColor,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = label,
                style = MaterialTheme.typography.labelSmall,
                fontWeight = FontWeight.Bold,
                color = contentColor,
                modifier =
                    Modifier
                        .clickable(onClick = onToggle)
                        .padding(start = 8.dp, top = 4.dp, end = 6.dp, bottom = 4.dp),
            )
            Box(
                modifier =
                    Modifier
                        .clickable(onClick = onConfigure)
                        .padding(start = 4.dp, top = 3.dp, end = 7.dp, bottom = 3.dp),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = Icons.Default.Tune,
                    contentDescription = contentDescription,
                    tint = contentColor,
                    modifier = Modifier.size(14.dp),
                )
            }
        }
    }
}

@Composable
private fun PortsPolicyDialog(
    app: AppEntry,
    portPolicy: PortPolicy?,
    onDismiss: () -> Unit,
    onSave: (PortPolicy?) -> Unit,
) {
    val initialMode = remember(app.packageName, portPolicy) { portPolicy.toUiMode() }
    var mode by remember(app.packageName, portPolicy) { mutableStateOf(initialMode) }
    val defaultPreset = PORT_PRESET_COMMON_PROXY
    var selectedPreset by remember(app.packageName, portPolicy) {
        mutableStateOf(portPolicy?.preset?.takeIf { portPreset(it) != null } ?: defaultPreset)
    }
    var customRules by remember(app.packageName, portPolicy) {
        mutableStateOf(
            (
                portPolicy?.takeIf { it.toUiMode() == PortPolicyUiMode.Custom }?.rules
                    ?: portPreset(selectedPreset)?.rules
                    ?: listOf(PortRule(start = 1080))
            ).map(PortRule::toEditable),
        )
    }

    val parsedCustomRules = customRules.mapNotNull(EditablePortRule::toPortRuleOrNull)
    val customRulesValid = parsedCustomRules.size == customRules.size && parsedCustomRules.isNotEmpty()
    val policyToSave =
        when (mode) {
            PortPolicyUiMode.All -> {
                null
            }

            PortPolicyUiMode.Preset -> {
                portPolicyForPreset(selectedPreset)
            }

            PortPolicyUiMode.Custom -> {
                if (customRulesValid) {
                    PortPolicy(mode = PortPolicyMode.Custom, rules = normalizedPortRules(parsedCustomRules))
                } else {
                    null
                }
            }
        }
    val canSave = mode != PortPolicyUiMode.Custom || customRulesValid

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.ports_policy_title)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    text = app.label,
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(
                    text = app.packageName,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                PortPolicyModeOption(
                    title = stringResource(R.string.ports_policy_all_title),
                    body = stringResource(R.string.ports_policy_all_body),
                    selected = mode == PortPolicyUiMode.All,
                    onClick = { mode = PortPolicyUiMode.All },
                )
                PortPolicyModeOption(
                    title = stringResource(R.string.ports_policy_preset_title),
                    body =
                        stringResource(
                            R.string.ports_policy_preset_body,
                            stringResource(R.string.ports_policy_common_proxy),
                            portRulesSummary(portPreset(defaultPreset)?.rules.orEmpty()),
                        ),
                    selected = mode == PortPolicyUiMode.Preset,
                    onClick = {
                        selectedPreset = defaultPreset
                        mode = PortPolicyUiMode.Preset
                    },
                )
                PortPolicyModeOption(
                    title = stringResource(R.string.ports_policy_custom_title),
                    body = stringResource(R.string.ports_policy_custom_body),
                    selected = mode == PortPolicyUiMode.Custom,
                    onClick = { mode = PortPolicyUiMode.Custom },
                )
                if (mode == PortPolicyUiMode.Custom) {
                    LazyColumn(
                        modifier = Modifier.heightIn(max = 260.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        items(customRules.indices.toList(), key = { it }) { index ->
                            PortRuleEditorRow(
                                rule = customRules[index],
                                onChange = { updated ->
                                    customRules = customRules.toMutableList().also { it[index] = updated }
                                },
                                onDelete = {
                                    customRules = customRules.filterIndexed { idx, _ -> idx != index }
                                },
                            )
                        }
                    }
                    TextButton(
                        onClick = { customRules = customRules + EditablePortRule(start = "1080") },
                    ) {
                        Icon(
                            imageVector = Icons.Default.Add,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        Text(stringResource(R.string.ports_policy_add_rule))
                    }
                    if (!customRulesValid) {
                        Text(
                            text = stringResource(R.string.ports_policy_invalid),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = canSave,
                onClick = { if (canSave) onSave(policyToSave) },
            ) {
                Text(stringResource(R.string.btn_save))
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
private fun PortPolicyModeOption(
    title: String,
    body: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(6.dp),
        color = if (selected) color.copy(alpha = 0.08f) else MaterialTheme.colorScheme.surface,
        border = BorderStroke(1.dp, color.copy(alpha = if (selected) 0.55f else 0.25f)),
    ) {
        Row(
            modifier =
                Modifier
                    .clickable(onClick = onClick)
                    .padding(10.dp),
            verticalAlignment = Alignment.Top,
        ) {
            RadioButton(selected = selected, onClick = onClick)
            Column(
                modifier = Modifier.padding(start = 8.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    text = title,
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold,
                )
                Text(
                    text = body,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun PortRuleEditorRow(
    rule: EditablePortRule,
    onChange: (EditablePortRule) -> Unit,
    onDelete: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        TextButton(onClick = { onChange(rule.copy(protocol = rule.protocol.next())) }) {
            Text(protocolLabel(rule.protocol))
        }
        OutlinedTextField(
            value = rule.start,
            onValueChange = { onChange(rule.copy(start = it.filter { ch -> ch.isDigit() })) },
            label = { Text(stringResource(R.string.ports_policy_start)) },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            singleLine = true,
            modifier = Modifier.weight(1f),
        )
        OutlinedTextField(
            value = rule.end,
            onValueChange = { onChange(rule.copy(end = it.filter { ch -> ch.isDigit() })) },
            label = { Text(stringResource(R.string.ports_policy_end)) },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            singleLine = true,
            modifier = Modifier.weight(1f),
        )
        IconButton(onClick = onDelete) {
            Icon(
                imageVector = Icons.Default.Delete,
                contentDescription = stringResource(R.string.ports_policy_delete_rule),
            )
        }
    }
}

@Composable
private fun HooksDialog(
    app: AppEntry,
    title: String,
    hookEntries: List<HookIds.Hook>,
    selectedHooks: List<String>?,
    roleEnabled: Boolean,
    onDismiss: () -> Unit,
    onSave: (List<String>?) -> Unit,
) {
    val hookNames = remember(hookEntries) { hookEntries.map { it.hookName } }
    // `selectedHooks == null` is ambiguous: it means BOTH "role on with all
    // hooks" and "role off". Disambiguate with roleEnabled so opening the dialog
    // on a disabled role starts with nothing checked — otherwise it would show
    // every hook pre-checked and Save would silently turn the role into a full
    // target (resolveHookSelection(all) -> null -> enabled).
    var selected by remember(app.packageName, selectedHooks, roleEnabled, hookNames) {
        mutableStateOf(
            when {
                selectedHooks != null -> selectedHooks.toSet()
                roleEnabled -> hookNames.toSet()
                else -> emptySet()
            },
        )
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    text = app.label,
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(
                    text = app.packageName,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                // Group hooks by detection method, with a header explaining what
                // the method is and how an app uses it to detect a VPN — same
                // taxonomy/wording as the Statistics per-hook detail.
                val grouped =
                    remember(hookEntries) {
                        hookEntries.groupBy { DetectionMethod.of(it) }.toSortedMap(compareBy { it.ordinal })
                    }
                LazyColumn(modifier = Modifier.heightIn(max = 460.dp)) {
                    grouped.forEach { (method, hooks) ->
                        item(key = "header_${method.name}") {
                            MethodHeader(method)
                        }
                        items(hooks, key = { it.hookName }) { hook ->
                            HookRow(
                                hook = hook,
                                checked = hook.hookName in selected,
                                onCheckedChange = { enabled ->
                                    selected =
                                        if (enabled) {
                                            selected + hook.hookName
                                        } else {
                                            selected - hook.hookName
                                        }
                                },
                            )
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    onSave(resolveHookSelection(hookNames, selected))
                },
            ) {
                Text(stringResource(R.string.btn_save))
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
private fun MethodHeader(method: DetectionMethod) {
    Column(modifier = Modifier.padding(top = 10.dp, bottom = 2.dp)) {
        Text(
            text = stringResource(method.labelRes),
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.Bold,
        )
        Text(
            text = stringResource(method.descriptionRes),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun HookRow(
    hook: HookIds.Hook,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    Row(
        modifier =
            Modifier
                .clickable { onCheckedChange(!checked) }
                .fillMaxWidth()
                .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = checked, onCheckedChange = onCheckedChange)
        // The friendly method name + explanation live in the group header; the
        // row just needs the precise hook (its technical note) to disambiguate.
        Text(
            text = hook.note,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(start = 8.dp),
        )
    }
}
