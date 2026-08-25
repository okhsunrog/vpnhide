package dev.okhsunrog.vpnhide.startup

import android.Manifest
import android.os.Bundle
import android.os.Process
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.BarChart
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.lerp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import dev.okhsunrog.vpnhide.AgentControlBridge
import dev.okhsunrog.vpnhide.BackgroundUpdateChecks
import dev.okhsunrog.vpnhide.DashboardCache
import dev.okhsunrog.vpnhide.DashboardLoadingState
import dev.okhsunrog.vpnhide.DashboardScreen
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.SelfTargetFailureKind
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.diagnostics.GroundTruthProbe
import dev.okhsunrog.vpnhide.diagnostics.RoutingGateCache
import dev.okhsunrog.vpnhide.diagnostics.VpnTransportWatcher
import dev.okhsunrog.vpnhide.picker.AppListCache
import dev.okhsunrog.vpnhide.picker.AppSearchTopBar
import dev.okhsunrog.vpnhide.picker.ProtectionScreen
import dev.okhsunrog.vpnhide.picker.TargetFilterChips
import dev.okhsunrog.vpnhide.picker.TargetListSortMode
import dev.okhsunrog.vpnhide.picker.TargetsCache
import dev.okhsunrog.vpnhide.picker.isMainAppProfile
import dev.okhsunrog.vpnhide.settings.AppSettings
import dev.okhsunrog.vpnhide.settings.DiagnosticsSettingsScreen
import dev.okhsunrog.vpnhide.settings.LocalSettingsInteractor
import dev.okhsunrog.vpnhide.settings.LocalSettingsState
import dev.okhsunrog.vpnhide.settings.RepositorySettingsInteractor
import dev.okhsunrog.vpnhide.settings.SettingsRepository
import dev.okhsunrog.vpnhide.settings.SettingsScreen
import dev.okhsunrog.vpnhide.shouldRequestUpdateNotificationPermission
import dev.okhsunrog.vpnhide.statistics.StatisticsCache
import dev.okhsunrog.vpnhide.statistics.StatisticsScreen
import dev.okhsunrog.vpnhide.suExec
import dev.okhsunrog.vpnhide.ui.components.BlockingErrorCard
import dev.okhsunrog.vpnhide.ui.components.ButtonSpinner
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard
import dev.okhsunrog.vpnhide.ui.components.pulse
import dev.okhsunrog.vpnhide.ui.components.rememberHapticTick
import dev.okhsunrog.vpnhide.ui.theme.AppColors
import dev.okhsunrog.vpnhide.ui.theme.AppEasing
import dev.okhsunrog.vpnhide.ui.theme.VpnHideTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        StartupTrace.mark("activity_on_create")
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val mainProfile = isMainAppProfile(Process.myUid())
        if (mainProfile) {
            RootSnapshotCache.setRuntimeProbeSource(GroundTruthProbe.prepare(this)?.absolutePath)
            // Load canonical debug state before runtime work so first suExec and dashboard bootstrap agree.
            VpnHideLog.init()
            // Process-scoped, registered once: auto-refreshes RoutingGateCache on VPN
            // up/down so Diagnostics/Dashboard/export/logcat react without a manual
            // re-check. Safe before RoutingGateCache has ever loaded — its trigger is
            // a no-op refreshInPlace guarded by runCatching until a screen seeds it.
            VpnTransportWatcher.start(this)
        }
        setContent {
            VpnHideApp(mainProfile)
        }
    }
}

private sealed interface RootState {
    data object Granted : RootState

    data object Denied : RootState
}

private fun checkRootAccess(): Boolean {
    val (exitCode, stdout) = suExec("id")
    return exitCode == 0 && stdout.contains("uid=0")
}

@Composable
private fun BackgroundUpdatePromptDialog(
    onEnable: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.background_update_prompt_title)) },
        text = { Text(stringResource(R.string.background_update_prompt_body)) },
        confirmButton = {
            TextButton(onClick = onEnable) {
                Text(stringResource(R.string.background_update_prompt_enable))
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text(stringResource(R.string.background_update_prompt_not_now))
            }
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun VpnHideApp(mainProfile: Boolean = isMainAppProfile(Process.myUid())) {
    if (!mainProfile) {
        VpnHideTheme {
            SecondaryProfileBlockedScreen()
        }
        return
    }

    val context = LocalContext.current
    val settingsRepository = remember(context) { SettingsRepository(context.applicationContext) }
    val loadedSettings by settingsRepository.settings.collectAsState(initial = null)
    val settingsLoaded = loadedSettings != null
    val settings = loadedSettings ?: AppSettings()
    val settingsScope = rememberCoroutineScope()
    val settingsInteractor =
        remember(settingsRepository, settingsScope) {
            RepositorySettingsInteractor(settingsRepository, settingsScope)
        }
    val notificationPermissionLauncher =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
            // Work scheduling is controlled by the DataStore setting. If the user
            // denies notifications, the worker still runs but skips notification
            // delivery until permission is granted later.
        }

    fun requestUpdateNotificationsIfNeeded() {
        if (shouldRequestUpdateNotificationPermission(context)) {
            notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    LaunchedEffect(settings.agentControlEnabled) {
        withContext(Dispatchers.IO) {
            AgentControlBridge.setEnabled(context.applicationContext, settings.agentControlEnabled)
        }
    }
    LaunchedEffect(
        settingsLoaded,
        settings.backgroundUpdateChecksConfigured,
        settings.backgroundUpdateChecksEnabled,
    ) {
        if (settingsLoaded) {
            withContext(Dispatchers.IO) {
                BackgroundUpdateChecks.sync(context.applicationContext, settings)
            }
        }
    }

    CompositionLocalProvider(
        LocalSettingsState provides settings,
        LocalSettingsInteractor provides settingsInteractor,
    ) {
        VpnHideTheme {
            var rootState by remember { mutableStateOf<RootState?>(null) }
            val rootCheckScope = rememberCoroutineScope()
            // Re-probe root without relaunching the app: clears to the loading
            // state, then re-runs the check. Lets the no-root gate offer a
            // "Check again" button after the user grants root in their manager.
            val probeRoot: () -> Unit = {
                rootState = null
                rootCheckScope.launch {
                    val granted = withContext(Dispatchers.IO) { checkRootAccess() }
                    rootState = if (granted) RootState.Granted else RootState.Denied
                    StartupTrace.mark("root_check_done")
                }
            }

            LaunchedEffect(Unit) { probeRoot() }

            when (rootState) {
                null -> {
                    StartupLoadingScreen()
                }

                RootState.Denied -> {
                    LaunchedEffect(Unit) { StartupTrace.rootDeniedReady() }
                    RootDeniedScreen(onRecheck = probeRoot)
                }

                RootState.Granted -> {
                    // Only prompt about background update checks once the app is
                    // actually usable (root granted) — not over the loading or
                    // no-root gate, where nothing else works yet.
                    var showBackgroundUpdatePrompt by remember { mutableStateOf(false) }
                    LaunchedEffect(settingsLoaded, settings.backgroundUpdateChecksConfigured) {
                        showBackgroundUpdatePrompt =
                            settingsLoaded && !settings.backgroundUpdateChecksConfigured
                    }
                    if (showBackgroundUpdatePrompt) {
                        BackgroundUpdatePromptDialog(
                            onEnable = {
                                showBackgroundUpdatePrompt = false
                                settingsInteractor.setBackgroundUpdateChecksEnabled(true)
                                requestUpdateNotificationsIfNeeded()
                            },
                            onDismiss = {
                                showBackgroundUpdatePrompt = false
                                settingsInteractor.setBackgroundUpdateChecksEnabled(false)
                            },
                        )
                    }
                    MainScreen()
                }
            }
        }
    }
}

@Composable
private fun SecondaryProfileBlockedScreen() {
    StartupBlockingScreen(
        title = stringResource(R.string.secondary_profile_error_title),
        body = stringResource(R.string.secondary_profile_error_message),
    )
}

private enum class Tab { Dashboard, Protection, Statistics }

private data class RefreshContext(
    val loading: Boolean,
    val onRefresh: () -> Unit,
)

@Composable
private fun tabLabel(tab: Tab): String =
    when (tab) {
        Tab.Dashboard -> stringResource(R.string.tab_dashboard)
        Tab.Statistics -> stringResource(R.string.tab_statistics)
        Tab.Protection -> stringResource(R.string.tab_protection)
    }

@Composable
private fun AppTopBarTitle(currentTab: Tab) {
    val tabLabel = tabLabel(currentTab)
    // The full-size brand block (logo + wordmark) wants ~190dp. Material3's
    // TopAppBar hands the title only the width left over after the action
    // buttons, so on very narrow / high-density screens that slot shrinks.
    // Scale the logo and the two text lines down together (never below 70%) so
    // the brand stays on a single line instead of wrapping.
    BoxWithConstraints {
        val scale = (maxWidth.value / 200f).coerceIn(0.6f, 1f)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                painter = painterResource(R.drawable.topbar_mark),
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.size(46.dp * scale),
            )
            Spacer(Modifier.width(12.dp * scale))
            Column {
                Text(
                    text = stringResource(R.string.app_name),
                    style = MaterialTheme.typography.headlineSmall,
                    fontSize = MaterialTheme.typography.headlineSmall.fontSize * scale,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1,
                )
                Text(
                    text = tabLabel,
                    style = MaterialTheme.typography.labelLarge,
                    fontSize = MaterialTheme.typography.labelLarge.fontSize * scale,
                    color = MaterialTheme.colorScheme.primary,
                    maxLines = 1,
                )
            }
        }
    }
}

/**
 * Brand block (logo + wordmark + tab label) for the main header, able to grow.
 * [progress] 0 = compact (single-line bar, same as everywhere), 1 = airy (big
 * logo dropped below the pinned action buttons — used only on the Dashboard on
 * tall screens). It also keeps the narrow-screen down-scaling of the compact
 * state so nothing wraps on small widths.
 */
@Composable
private fun AppHeaderBrand(
    currentTab: Tab,
    progress: Float,
    modifier: Modifier = Modifier,
) {
    val tabLabel = tabLabel(currentTab)
    BoxWithConstraints(modifier) {
        val narrowScale = (maxWidth.value / 200f).coerceIn(0.6f, 1f)
        val logoSize = lerp(38.dp * narrowScale, 58.dp, progress)
        val titleSize = MaterialTheme.typography.headlineSmall.fontSize * narrowScale * (1f + 0.28f * progress)
        val subSize = MaterialTheme.typography.labelLarge.fontSize * narrowScale * (1f + 0.12f * progress)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                painter = painterResource(R.drawable.topbar_mark),
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.size(logoSize),
            )
            Spacer(Modifier.width(lerp(12.dp, 16.dp, progress)))
            Column {
                Text(
                    text = stringResource(R.string.app_name),
                    style = MaterialTheme.typography.headlineSmall,
                    fontSize = titleSize,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                )
                Text(
                    text = tabLabel,
                    style = MaterialTheme.typography.labelLarge,
                    fontSize = subSize,
                    color = MaterialTheme.colorScheme.primary,
                    maxLines = 1,
                )
            }
        }
    }
}

@Composable
private fun TopBarActionButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    active: Boolean = false,
    content: @Composable () -> Unit,
) {
    val containerColor =
        if (active) {
            MaterialTheme.colorScheme.primaryContainer
        } else {
            AppColors.toolbarActionContainer
        }
    val contentColor =
        if (active) {
            MaterialTheme.colorScheme.onPrimaryContainer
        } else {
            MaterialTheme.colorScheme.onSurfaceVariant
        }
    FilledTonalIconButton(
        onClick = onClick,
        modifier = modifier.size(44.dp),
        enabled = enabled,
        colors =
            IconButtonDefaults.filledTonalIconButtonColors(
                containerColor = containerColor,
                contentColor = contentColor,
                disabledContainerColor = containerColor.copy(alpha = 0.48f),
                disabledContentColor = contentColor.copy(alpha = 0.55f),
            ),
        content = content,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MainScreen() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val appContext = context.applicationContext
    val startupCoordinator = remember(appContext) { StartupCoordinator(appContext) }
    val settings = LocalSettingsState.current
    val settingsInteractor = LocalSettingsInteractor.current
    var currentTab by remember { mutableStateOf(Tab.Dashboard) }
    var searchQuery by remember { mutableStateOf("") }
    var searchActive by remember { mutableStateOf(false) }
    var showSystem by remember { mutableStateOf(false) }
    var showRussianOnly by remember { mutableStateOf(false) }
    var targetSortMode by remember { mutableStateOf(TargetListSortMode.ConfiguredFirst) }
    val appListLoading by AppListCache.loading.collectAsState()
    val targetsLoading by TargetsCache.loading.collectAsState()
    val dashboardLoading by DashboardCache.loading.collectAsState()
    val statisticsLoading by StatisticsCache.loading.collectAsState()
    val dashboardState by DashboardCache.state.collectAsState()
    val dashboardError by DashboardCache.error.collectAsState()
    val rootSnapshot by RootSnapshotCache.snapshot.collectAsState()
    val selfTargetState by startupCoordinator.selfTargetState.collectAsState()
    val selfNeedsRestart =
        (selfTargetState as? StartupSelfTargetState.Ready)?.selfNeedsRestart
    val selfTargetFailure = selfTargetState as? StartupSelfTargetState.Failed
    val refreshRestart = selfNeedsRestart ?: false

    LaunchedEffect(startupCoordinator) {
        startupCoordinator.prepareSelfTargets()
    }

    // Start the app-scoped caches as soon as the self-target preparation
    // is resolved. Keep that preparation first: it migrates/updates canonical
    // config and determines whether this app process needs a restart, so Dashboard
    // must not derive protection state from a stale answer. Protection still
    // prewarms before a normal tab switch, but without racing the self-target
    // root shell.
    LaunchedEffect(selfNeedsRestart) {
        val r = selfNeedsRestart ?: return@LaunchedEffect
        startupCoordinator.ensureInitialCaches(scope, r)
    }

    // Protection depends on the same root snapshot as Dashboard. Let
    // Dashboard own the initial root snapshot so a transient shell timeout
    // cannot make startup immediately do a second expensive retry. As soon
    // as that shared snapshot exists, TargetsCache parses it from memory and
    // Protection is still prewarmed before a normal tab switch.
    LaunchedEffect(selfNeedsRestart, rootSnapshot) {
        VpnHideLog.setFromRootSnapshot(rootSnapshot)
        startupCoordinator.ensureProtectionCacheAfterRootSnapshot(scope, selfNeedsRestart, rootSnapshot)
    }

    // Mark startup readiness once the main shell has shown either usable
    // dashboard data or a terminal load error. Users see the dashboard loading
    // state in place while this happens.
    val uiReady = startupCoordinator.isUiReady(dashboardState, dashboardError)
    var startupTraceMarked by remember { mutableStateOf(false) }
    LaunchedEffect(uiReady) {
        if (uiReady && !startupTraceMarked) {
            startupTraceMarked = true
            StartupTrace.dashboardReady()
        }
    }

    // Kick the update check once (silently) on first launch, and again
    // on ON_RESUME if it's been a while. Listener lives as long as
    // MainScreen is composed.
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer =
            LifecycleEventObserver { _, event ->
                if (event == Lifecycle.Event.ON_RESUME) {
                    startupCoordinator.ensureUpdateFresh(scope)
                    // Belt-and-suspenders re-probe of the routing gate on foreground
                    // return (throttled), covering the case where the background VPN
                    // transport callback was frozen/missed while we were away — e.g. the
                    // user left to toggle their VPN in another app and came back.
                    RoutingGateCache.refreshIfStale(scope)
                }
            }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    LaunchedEffect(currentTab) {
        if (currentTab != Tab.Protection) {
            searchActive = false
            searchQuery = ""
        }
    }
    BackHandler(enabled = searchActive && currentTab == Tab.Protection) {
        searchActive = false
        searchQuery = ""
    }

    var showSettings by remember { mutableStateOf(false) }
    if (showSettings) {
        BackHandler { showSettings = false }
        SettingsScreen(
            selfNeedsRestart = selfNeedsRestart,
            onBack = { showSettings = false },
        )
        return
    }

    // Full-screen diagnostics overlay, reachable from a Dashboard message's
    // "Details" button (the same screen Settings → Diagnostics opens).
    var showDiagnostics by remember { mutableStateOf(false) }
    if (showDiagnostics) {
        DiagnosticsSettingsScreen(
            selfNeedsRestart = selfNeedsRestart,
            onBack = { showDiagnostics = false },
        )
        return
    }

    Scaffold(
        containerColor = AppColors.screenBackground,
        topBar = {
            if (searchActive && currentTab == Tab.Protection) {
                AppSearchTopBar(
                    query = searchQuery,
                    onQueryChange = { searchQuery = it },
                    onClose = {
                        searchActive = false
                        searchQuery = ""
                    },
                )
            } else {
                // Dashboard on a tall screen gets an "airy" header: the brand
                // grows and drops below the (fixed) action buttons. Everywhere
                // else — and on short screens — it stays the compact single row.
                // The whole thing morphs via `headerProgress` on tab switch.
                val airy =
                    currentTab == Tab.Dashboard &&
                        LocalConfiguration.current.screenHeightDp >= 760
                val headerProgress by animateFloatAsState(
                    targetValue = if (airy) 1f else 0f,
                    animationSpec = tween(durationMillis = 300),
                    label = "headerAiry",
                )
                // Room the brand must leave for the pinned buttons in the compact
                // state (2 actions normally, 3 on Protection with Search).
                val actionReserve = ((if (currentTab == Tab.Protection) 3 else 2) * 52 + 24).dp
                Surface(color = AppColors.topBarContainer) {
                    Box(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .statusBarsPadding(),
                    ) {
                        AppHeaderBrand(
                            currentTab = currentTab,
                            progress = headerProgress,
                            modifier =
                                Modifier
                                    .align(Alignment.TopStart)
                                    .padding(
                                        start = 16.dp,
                                        top = lerp(13.dp, 32.dp, headerProgress),
                                        bottom = lerp(13.dp, 22.dp, headerProgress),
                                        end = lerp(actionReserve, 12.dp, headerProgress),
                                    ),
                        )
                        Row(
                            modifier =
                                Modifier
                                    .align(Alignment.TopEnd)
                                    .padding(top = 10.dp, end = 16.dp),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            // Refresh is contextual: Protection refreshes
                            // the app list, Dashboard refreshes the dashboard
                            // state + update check.
                            val refreshContext =
                                when (currentTab) {
                                    Tab.Dashboard -> {
                                        RefreshContext(
                                            loading = dashboardLoading,
                                            onRefresh = {
                                                startupCoordinator.refreshDashboard(scope, refreshRestart)
                                            },
                                        )
                                    }

                                    Tab.Statistics -> {
                                        RefreshContext(
                                            loading = statisticsLoading,
                                            onRefresh = {
                                                StatisticsCache.refresh(scope)
                                            },
                                        )
                                    }

                                    Tab.Protection -> {
                                        RefreshContext(
                                            loading = appListLoading || targetsLoading,
                                            onRefresh = {
                                                startupCoordinator.refreshProtection(scope)
                                            },
                                        )
                                    }
                                }
                            if (currentTab == Tab.Protection) {
                                // Filters (system / RU / sort) now live as chips in
                                // the Apps list itself — see TargetFilterChips. Only
                                // Search stays in the bar, so the 4-button crowding
                                // on narrow/high-density screens is gone.
                                TopBarActionButton(onClick = { searchActive = true }) {
                                    Icon(
                                        Icons.Default.Search,
                                        contentDescription = null,
                                    )
                                }
                            }
                            TopBarActionButton(
                                onClick = refreshContext.onRefresh,
                                enabled = !refreshContext.loading,
                            ) {
                                if (refreshContext.loading) {
                                    ButtonSpinner(size = 20.dp)
                                } else {
                                    Icon(
                                        Icons.Default.Refresh,
                                        contentDescription = stringResource(R.string.action_refresh),
                                    )
                                }
                            }
                            TopBarActionButton(
                                onClick = {
                                    showSettings = true
                                    if (!settings.settingsHintSeen) {
                                        scope.launch { settingsInteractor.setSettingsHintSeen(true) }
                                    }
                                },
                                modifier =
                                    Modifier.pulse(
                                        enabled = !settings.settingsHintSeen && settings.animationsEnabled,
                                    ),
                            ) {
                                Icon(
                                    Icons.Default.Settings,
                                    contentDescription = stringResource(R.string.action_settings),
                                )
                            }
                        }
                    }
                }
            }
        },
        bottomBar = {
            val tabHaptic = rememberHapticTick()
            NavigationBar(
                containerColor = AppColors.navigationBarContainer,
                tonalElevation = 0.dp,
            ) {
                NavigationBarItem(
                    selected = currentTab == Tab.Dashboard,
                    onClick = {
                        tabHaptic()
                        currentTab = Tab.Dashboard
                    },
                    icon = { Icon(Icons.Default.Home, contentDescription = null) },
                    label = { Text(stringResource(R.string.tab_dashboard)) },
                )
                NavigationBarItem(
                    selected = currentTab == Tab.Protection,
                    onClick = {
                        tabHaptic()
                        currentTab = Tab.Protection
                    },
                    icon = { Icon(Icons.AutoMirrored.Filled.List, contentDescription = null) },
                    label = { Text(stringResource(R.string.tab_protection)) },
                )
                NavigationBarItem(
                    selected = currentTab == Tab.Statistics,
                    onClick = {
                        tabHaptic()
                        currentTab = Tab.Statistics
                    },
                    icon = { Icon(Icons.Default.BarChart, contentDescription = null) },
                    label = { Text(stringResource(R.string.tab_statistics)) },
                )
            }
        },
    ) { innerPadding ->
        val restart = selfNeedsRestart
        val preparationFailure = selfTargetFailure
        if (preparationFailure != null) {
            RootPreparationErrorScreen(
                kind = preparationFailure.kind,
                detail = preparationFailure.detail,
                modifier = Modifier.padding(innerPadding),
                onRetry = { startupCoordinator.retrySelfTargets(scope) },
            )
        } else if (restart == null) {
            DashboardLoadingState(modifier = Modifier.padding(innerPadding))
        } else {
            AnimatedContent(
                targetState = currentTab,
                transitionSpec = {
                    if (settings.animationsEnabled) {
                        // ImageToolbox's pervasive AnimatedContent transition: a
                        // fade-through with a slight scale, in place (no slide).
                        // The old tab's rows dissolve and shrink while the new
                        // tab's rows fade and grow into the same positions, so
                        // one set of rows reads as morphing into the other.
                        (
                            fadeIn(tween(300, easing = AppEasing.Alpha)) +
                                scaleIn(tween(400, easing = AppEasing.Scale), initialScale = 0.92f)
                        ) togetherWith (
                            fadeOut(tween(300, easing = AppEasing.Alpha)) +
                                scaleOut(tween(400, easing = AppEasing.Scale), targetScale = 0.92f)
                        )
                    } else {
                        EnterTransition.None togetherWith ExitTransition.None
                    }
                },
                label = "tabContent",
            ) { tab ->
                when (tab) {
                    Tab.Dashboard -> {
                        DashboardScreen(
                            selfNeedsRestart = restart,
                            onOpenDiagnostics = { showDiagnostics = true },
                            modifier = Modifier.padding(innerPadding),
                        )
                    }

                    Tab.Statistics -> {
                        StatisticsScreen(
                            modifier = Modifier.padding(innerPadding),
                        )
                    }

                    Tab.Protection -> {
                        ProtectionScreen(
                            searchQuery = searchQuery,
                            showSystem = showSystem,
                            showRussianOnly = showRussianOnly,
                            sortMode = targetSortMode,
                            onToggleSystem = { showSystem = !showSystem },
                            onToggleRussianOnly = { showRussianOnly = !showRussianOnly },
                            onSortModeChange = { targetSortMode = it },
                            modifier = Modifier.padding(innerPadding),
                        )
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun StartupLoadingScreen() {
    Scaffold(
        containerColor = AppColors.screenBackground,
        topBar = {
            TopAppBar(
                title = { AppTopBarTitle(Tab.Dashboard) },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = AppColors.topBarContainer,
                        scrolledContainerColor = AppColors.topBarScrolledContainer,
                        titleContentColor = MaterialTheme.colorScheme.onSurface,
                    ),
            )
        },
    ) { innerPadding ->
        DashboardLoadingState(modifier = Modifier.padding(innerPadding))
    }
}

private fun selfTargetErrorBodyRes(kind: SelfTargetFailureKind): Int =
    when (kind) {
        SelfTargetFailureKind.RootUnavailable -> R.string.self_targets_error_body_root
        SelfTargetFailureKind.IncompleteData -> R.string.self_targets_error_body_incomplete
        SelfTargetFailureKind.ConfigWriteFailed -> R.string.self_targets_error_body_write
        SelfTargetFailureKind.Unknown -> R.string.self_targets_error_body_unknown
    }

@Composable
private fun RootPreparationErrorScreen(
    kind: SelfTargetFailureKind,
    detail: String,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier =
            modifier
                .fillMaxSize()
                .padding(24.dp),
        contentAlignment = Alignment.Center,
    ) {
        BlockingErrorCard(
            icon = Icons.Default.ErrorOutline,
            title = stringResource(R.string.self_targets_error_title),
            body = stringResource(selfTargetErrorBodyRes(kind)),
            detail = detail.takeIf { it.isNotBlank() },
            actionLabel = stringResource(R.string.vpn_off_retry),
            onAction = onRetry,
        )
    }
}

@Composable
private fun RootDeniedScreen(onRecheck: () -> Unit) {
    StartupBlockingScreen(
        title = stringResource(R.string.root_error_title),
        body = stringResource(R.string.root_error_message),
        actionLabel = stringResource(R.string.root_error_recheck),
        onAction = onRecheck,
    )
}
