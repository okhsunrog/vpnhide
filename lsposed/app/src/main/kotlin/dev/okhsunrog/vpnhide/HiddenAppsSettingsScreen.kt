package dev.okhsunrog.vpnhide

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SearchBar
import androidx.compose.material3.SearchBarDefaults
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.theme.AppColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun HiddenAppsSettingsScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val targets by TargetsCache.snapshot.collectAsState()
    val apps by AppListCache.apps.collectAsState()
    val userNames by AppListCache.userNames.collectAsState()
    val snackbarHostState = remember { SnackbarHostState() }
    var filter by remember { mutableStateOf(HiddenAppsFilter.All) }
    var query by remember { mutableStateOf("") }
    var searchActive by remember { mutableStateOf(false) }
    var saving by remember { mutableStateOf(false) }
    var snackMessage by remember { mutableStateOf<String?>(null) }
    val savedMessage = stringResource(R.string.settings_auto_hide_saved)
    val failedMessage = stringResource(R.string.settings_auto_hide_failed)

    LaunchedEffect(Unit) {
        TargetsCache.ensureLoaded(scope, context)
        AppListCache.ensureLoaded(scope, context)
    }
    LaunchedEffect(snackMessage) {
        snackMessage?.let {
            snackbarHostState.showSnackbar(it, duration = SnackbarDuration.Short)
            snackMessage = null
        }
    }

    val canonical = targets?.let(::buildCanonicalConfigFromTargetsSnapshot)
    val appList = apps
    val initialManual =
        remember(canonical, context.packageName) {
            canonical?.let { manualHiddenPackages(it, context.packageName) }.orEmpty()
        }
    val initialExcluded =
        remember(canonical, context.packageName) {
            canonical?.settings?.autoHideExcludedPackages.orEmpty() - context.packageName
        }
    var selectedManual by remember(initialManual) { mutableStateOf(initialManual) }
    var excludedPackages by remember(initialExcluded) { mutableStateOf(initialExcluded) }
    val signals = remember(appList) { appList.orEmpty().map(AppSummary::toAutoHideSignal) }
    val baseStates =
        remember(canonical, context.packageName, signals) {
            canonical?.let { hiddenAppStates(it, context.packageName, signals) }.orEmpty()
        }
    val visiblePackageScope =
        remember(appList, baseStates) {
            appList.orEmpty().mapTo(mutableSetOf()) { it.packageName } +
                baseStates.mapTo(mutableSetOf()) { it.packageName }
        }
    val draftConfig =
        remember(canonical, context.packageName, visiblePackageScope, selectedManual, excludedPackages, signals) {
            canonical?.let {
                updateHiddenAppsConfig(
                    config = it,
                    selfPkg = context.packageName,
                    visiblePackages = visiblePackageScope,
                    selectedManualHiddenPackages = selectedManual,
                    excludedPackages = excludedPackages,
                    signals = signals,
                )
            }
        }
    val states =
        remember(draftConfig, context.packageName, signals) {
            draftConfig?.let { hiddenAppStates(it, context.packageName, signals) }.orEmpty()
        }
    val summary = remember(states) { hiddenAppsSummary(states) }
    val appsByPackage = remember(appList) { appList.orEmpty().associateBy { it.packageName } }
    val labelsByPackage = remember(appList) { appList.orEmpty().associate { it.packageName to it.label } }
    val visibleStates =
        remember(baseStates, states, filter, query, labelsByPackage) {
            visibleHiddenAppStates(
                savedStates = baseStates,
                draftStates = states,
                filter = filter,
                searchQuery = query,
                labelsByPackage = labelsByPackage,
            )
        }
    val dirty = selectedManual != initialManual || excludedPackages != initialExcluded

    BackHandler {
        if (searchActive) {
            searchActive = false
            query = ""
        } else {
            onBack()
        }
    }

    Scaffold(
        containerColor = AppColors.screenBackground,
        topBar = {
            if (searchActive) {
                SearchBar(
                    inputField = {
                        SearchBarDefaults.InputField(
                            query = query,
                            onQueryChange = { query = it },
                            onSearch = {},
                            expanded = false,
                            onExpandedChange = {},
                            placeholder = { Text(stringResource(R.string.search_placeholder)) },
                            leadingIcon = {
                                IconButton(onClick = {
                                    searchActive = false
                                    query = ""
                                }) {
                                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = null)
                                }
                            },
                            trailingIcon = {
                                if (query.isNotEmpty()) {
                                    IconButton(onClick = { query = "" }) {
                                        Icon(Icons.Default.Clear, contentDescription = null)
                                    }
                                }
                            },
                        )
                    },
                    expanded = false,
                    onExpandedChange = {},
                    modifier = Modifier.fillMaxWidth(),
                ) {}
            } else {
                TopAppBar(
                    title = { Text(stringResource(R.string.settings_hidden_apps)) },
                    navigationIcon = {
                        IconButton(onClick = onBack) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.action_back))
                        }
                    },
                    actions = {
                        FilledTonalIconButton(
                            onClick = { searchActive = true },
                            modifier =
                                Modifier
                                    .padding(end = 16.dp)
                                    .size(44.dp),
                            colors =
                                IconButtonDefaults.filledTonalIconButtonColors(
                                    containerColor = AppColors.toolbarActionContainer,
                                    contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                                ),
                        ) {
                            Icon(Icons.Default.Search, contentDescription = null)
                        }
                    },
                    colors =
                        TopAppBarDefaults.topAppBarColors(
                            containerColor = AppColors.topBarContainer,
                            titleContentColor = MaterialTheme.colorScheme.onSurface,
                            navigationIconContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                            actionIconContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                        ),
                )
            }
        },
        bottomBar = {
            HiddenAppsSaveBar(
                summary = summary,
                dirty = dirty,
                saving = saving,
                onSave = {
                    val config = canonical ?: return@HiddenAppsSaveBar
                    saving = true
                    scope.launch {
                        val exit =
                            withContext(Dispatchers.IO) {
                                writeHiddenApps(context, config, visiblePackageScope, selectedManual, excludedPackages, signals)
                            }
                        saving = false
                        snackMessage = if (exit == 0) savedMessage else failedMessage
                        if (exit == 0) {
                            TargetsCache.refreshAfterSave(scope, context)
                        }
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
    ) { padding ->
        if (canonical == null || appList == null) {
            Box(
                modifier = Modifier.fillMaxSize().padding(padding),
                contentAlignment = Alignment.Center,
            ) {
                CircularProgressIndicator()
            }
            return@Scaffold
        }

        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            HelpAccordion(
                prefKey = "hidden_apps",
                title = stringResource(R.string.apps_help_title),
            ) {
                Text(
                    text = stringResource(R.string.settings_hidden_help_body),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            HiddenAppsFilterRow(
                selected = filter,
                onSelected = { filter = it },
            )
            if (visibleStates.isEmpty()) {
                Box(
                    modifier = Modifier.fillMaxWidth().weight(1f),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = stringResource(R.string.settings_hidden_apps_empty),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            } else {
                LazyColumn(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    items(visibleStates, key = { it.packageName }) { state ->
                        HiddenAppRow(
                            state = state,
                            app = appsByPackage[state.packageName],
                            userNames = userNames,
                            checked = state.hidden,
                            onCheckedChange = { checked ->
                                val hasAutoSource = state.automatic || state.reasons.isNotEmpty()
                                if (checked) {
                                    excludedPackages = excludedPackages - state.packageName
                                    if (!hasAutoSource) selectedManual = selectedManual + state.packageName
                                } else {
                                    selectedManual = selectedManual - state.packageName
                                    if (hasAutoSource) {
                                        excludedPackages = excludedPackages + state.packageName
                                    }
                                }
                            },
                        )
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun HiddenAppsFilterRow(
    selected: HiddenAppsFilter,
    onSelected: (HiddenAppsFilter) -> Unit,
) {
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        HiddenAppsFilter.entries.forEach { filter ->
            FilterChip(
                selected = selected == filter,
                onClick = { onSelected(filter) },
                label = { Text(hiddenAppsFilterLabel(filter)) },
            )
        }
    }
}

@Composable
private fun hiddenAppsFilterLabel(filter: HiddenAppsFilter): String =
    when (filter) {
        HiddenAppsFilter.All -> stringResource(R.string.settings_hidden_filter_all)
        HiddenAppsFilter.Automatic -> stringResource(R.string.settings_hidden_filter_automatic)
        HiddenAppsFilter.Manual -> stringResource(R.string.settings_hidden_filter_manual)
        HiddenAppsFilter.Excluded -> stringResource(R.string.settings_hidden_filter_excluded)
    }

@Composable
private fun HiddenAppRow(
    state: HiddenAppState,
    app: AppSummary?,
    userNames: Map<Int, String>,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable { onCheckedChange(!checked) },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = checked, onCheckedChange = onCheckedChange)
        TargetRowShell(
            label = app?.label ?: state.packageName,
            packageName = state.packageName,
            icon = app?.icon,
            userIds = app?.userIds.orEmpty(),
            userNames = userNames,
            modifier = Modifier.weight(1f),
        ) {
            HiddenAppChips(state)
        }
    }
}

@Composable
private fun HiddenAppChips(state: HiddenAppState) {
    if (state.automatic) {
        HiddenAppMetaLabel(label = stringResource(R.string.settings_hidden_chip_auto), accent = StatusColors.successDot)
    }
    if (state.manual) {
        HiddenAppMetaLabel(label = stringResource(R.string.settings_hidden_chip_manual), accent = StatusColors.infoAccent)
    }
    if (state.excluded) {
        HiddenAppMetaLabel(label = stringResource(R.string.settings_hidden_chip_excluded), accent = StatusColors.warningAccent)
    }
    state.reasons.forEach { reason ->
        HiddenAppMetaLabel(label = autoHideReasonLabel(reason), accent = StatusColors.neutralAccent)
    }
    if (state.unavailable) {
        HiddenAppMetaLabel(label = stringResource(R.string.settings_hidden_chip_unavailable), accent = StatusColors.neutralAccent)
    }
}

@Composable
private fun HiddenAppMetaLabel(
    label: String,
    accent: Color,
) {
    Row(
        modifier = Modifier.padding(end = 8.dp, top = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier =
                Modifier
                    .size(7.dp)
                    .clip(CircleShape)
                    .background(accent),
        )
        Spacer(Modifier.width(5.dp))
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            fontWeight = FontWeight.SemiBold,
            color = accent,
        )
    }
}

@Composable
private fun autoHideReasonLabel(reason: AutoHideReason): String =
    when (reason) {
        AutoHideReason.VpnService -> stringResource(R.string.settings_hidden_reason_vpn_service)
        AutoHideReason.NameMatch -> stringResource(R.string.settings_hidden_reason_name)
    }

@Composable
private fun HiddenAppsSaveBar(
    summary: HiddenAppsSummary,
    dirty: Boolean,
    saving: Boolean,
    onSave: () -> Unit,
) {
    Surface(tonalElevation = 3.dp) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .navigationBarsPadding()
                    .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = stringResource(R.string.settings_hidden_apps_sub, summary.hidden, summary.automatic, summary.manual),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (summary.excluded > 0) {
                    Text(
                        text = stringResource(R.string.settings_hidden_apps_excluded_sub, summary.excluded),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            EnhancedButton(onClick = onSave, enabled = dirty && !saving) {
                if (saving) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(18.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.onPrimary,
                    )
                    Spacer(Modifier.width(8.dp))
                }
                Text(stringResource(R.string.btn_save))
            }
        }
    }
}

private fun writeHiddenApps(
    context: android.content.Context,
    base: CanonicalConfig,
    visiblePackages: Set<String>,
    selectedManualHiddenPackages: Set<String>,
    excludedPackages: Set<String>,
    signals: Collection<AppAutoHideSignal>,
): Int {
    val canonical =
        updateHiddenAppsConfig(
            config = base,
            selfPkg = context.packageName,
            visiblePackages = visiblePackages,
            selectedManualHiddenPackages = selectedManualHiddenPackages,
            excludedPackages = excludedPackages,
            signals = signals,
        )
    val cmd =
        listOf(
            buildCanonicalConfigWriteCommand(canonical),
            ConfigChannels.reconcileCommand(),
        ).joinToString(" && ")
    val (exit, _) = suExec(cmd)
    if (exit == 0) {
        RootSnapshotCache.invalidate()
        DashboardCache.invalidate()
    }
    return exit
}
