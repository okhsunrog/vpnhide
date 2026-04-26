package dev.okhsunrog.vpnhide

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.FilterList
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.util.concurrent.atomic.AtomicBoolean

class MainActivity : ComponentActivity() {
    private val splashReady = AtomicBoolean(false)

    override fun onCreate(savedInstanceState: Bundle?) {
        installSplashScreen().setKeepOnScreenCondition { !splashReady.get() }
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        // Load the user's debug-logging preference before anything else
        // runs so the first suExec + Dashboard reload honor it.
        VpnHideLog.init(applicationContext)
        // Re-propagate the persisted flag to the on-disk sinks. Reinstalling
        // a native module wipes its /data/adb/modules/<id>/ tree wholesale
        // (KSU/Magisk replace it from the zip), so the zygisk-side
        // debug_logging file disappears even though SharedPrefs still says
        // ON. Without this re-write zygisk would silently default to
        // error-only logging until the user toggled off-then-on. Cheap —
        // two `su` roundtrips on a background dispatcher.
        lifecycleScope.launch(Dispatchers.IO) {
            applyDebugLoggingRuntime(VpnHideLog.enabled)
        }
        setContent { VpnHideApp(onReady = { splashReady.set(true) }) }
    }
}

private sealed class RootState {
    data object Granted : RootState()

    data object Denied : RootState()
}

private fun checkRootAccess(): Boolean {
    val (exitCode, stdout) = suExec("id")
    return exitCode == 0 && stdout.contains("uid=0")
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun VpnHideApp(onReady: () -> Unit = {}) {
    val darkTheme = isSystemInDarkTheme()
    val colorScheme =
        if (android.os.Build.VERSION.SDK_INT >= 31) {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        } else {
            if (darkTheme) darkColorScheme() else lightColorScheme()
        }

    MaterialTheme(colorScheme = colorScheme) {
        var rootState by remember { mutableStateOf<RootState?>(null) }

        LaunchedEffect(Unit) {
            rootState =
                withContext(Dispatchers.IO) {
                    if (checkRootAccess()) RootState.Granted else RootState.Denied
                }
        }

        when (rootState) {
            // splash holds until root check completes
            null -> {
                Unit
            }

            RootState.Denied -> {
                // Drop splash — RootDeniedScreen has no async prerequisites.
                LaunchedEffect(Unit) { onReady() }
                RootDeniedScreen()
            }

            RootState.Granted -> {
                MainScreen(onReady = onReady)
            }
        }
    }
}

private enum class Tab { Dashboard, Protection, Diagnostics }

private data class RefreshContext(
    val loading: Boolean,
    val onRefresh: () -> Unit,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MainScreen(onReady: () -> Unit = {}) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var currentTab by remember { mutableStateOf(Tab.Dashboard) }
    var selfNeedsRestart by remember { mutableStateOf<Boolean?>(null) }
    var searchQuery by remember { mutableStateOf("") }
    var searchActive by remember { mutableStateOf(false) }
    var showSystem by remember { mutableStateOf(false) }
    var showRussianOnly by remember { mutableStateOf(false) }
    var showFilterMenu by remember { mutableStateOf(false) }
    val appListLoading by AppListCache.loading.collectAsState()
    val targetsLoading by TargetsCache.loading.collectAsState()
    val dashboardLoading by DashboardCache.loading.collectAsState()
    val dashboardState by DashboardCache.state.collectAsState()
    val refreshRestart = selfNeedsRestart ?: false

    LaunchedEffect(Unit) {
        selfNeedsRestart =
            withContext(Dispatchers.IO) {
                cleanupStaleZygiskStatus(context)
                ensureSelfInTargets(context.packageName)
            }
    }

    // Kick off both Protection caches as early as possible so tab
    // switches into Protection render instantly instead of paying the
    // per-screen pm + icon + root-shell cost each time.
    LaunchedEffect(Unit) {
        AppListCache.ensureLoaded(scope, context)
        TargetsCache.ensureLoaded(scope, context)
    }

    // Pre-warm Dashboard (needed for first frame) and Diagnostics (needed
    // when user switches to Diagnostics tab) as soon as selfNeedsRestart
    // is resolved. Dashboard prewarm here — not in DashboardScreen's own
    // LaunchedEffect — so it runs while the splash is still held, not
    // only after MainScreen has rendered.
    LaunchedEffect(selfNeedsRestart) {
        val r = selfNeedsRestart ?: return@LaunchedEffect
        DashboardCache.ensureLoaded(scope, context, r)
        if (!r) DiagnosticsCache.run(scope, context)
    }

    // Hold the splash screen until the first Dashboard frame can render
    // with real content. Without this, the user sees splash → brief
    // selfNeedsRestart-null spinner → brief Dashboard state-null spinner
    // → content, with each spinner swap being visible flicker.
    val uiReady = selfNeedsRestart != null && dashboardState != null
    LaunchedEffect(uiReady) {
        if (uiReady) onReady()
    }

    // Kick the update check once (silently) on first launch, and again
    // on ON_RESUME if it's been a while. Listener lives as long as
    // MainScreen is composed.
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer =
            LifecycleEventObserver { _, event ->
                if (event == Lifecycle.Event.ON_RESUME) {
                    UpdateCheckCache.ensureFresh(scope, BuildConfig.VERSION_NAME)
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

    Scaffold(
        topBar = {
            if (searchActive && currentTab == Tab.Protection) {
                SearchBar(
                    query = searchQuery,
                    onQueryChange = { searchQuery = it },
                    onSearch = {},
                    active = false,
                    onActiveChange = {},
                    placeholder = { Text(stringResource(R.string.search_placeholder)) },
                    leadingIcon = {
                        IconButton(onClick = {
                            searchActive = false
                            searchQuery = ""
                        }) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = null)
                        }
                    },
                    trailingIcon = {
                        if (searchQuery.isNotEmpty()) {
                            IconButton(onClick = { searchQuery = "" }) {
                                Icon(Icons.Default.Clear, contentDescription = null)
                            }
                        }
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) {}
            } else {
                TopAppBar(
                    title = { Text(stringResource(R.string.app_name)) },
                    colors =
                        TopAppBarDefaults.topAppBarColors(
                            containerColor = MaterialTheme.colorScheme.primaryContainer,
                            titleContentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                        ),
                    actions = {
                        // Refresh is contextual: Protection refreshes
                        // the app list, Dashboard refreshes the dashboard
                        // state + update check. Diagnostics has its own
                        // run buttons per-check, no top-bar refresh.
                        val refreshContext =
                            when (currentTab) {
                                Tab.Dashboard -> {
                                    RefreshContext(
                                        loading = dashboardLoading,
                                        onRefresh = {
                                            DashboardCache.refresh(scope, context, refreshRestart)
                                            UpdateCheckCache.refresh(scope, BuildConfig.VERSION_NAME)
                                        },
                                    )
                                }

                                Tab.Protection -> {
                                    RefreshContext(
                                        loading = appListLoading || targetsLoading,
                                        onRefresh = {
                                            AppListCache.refresh(scope, context)
                                            TargetsCache.refresh(scope, context)
                                        },
                                    )
                                }

                                Tab.Diagnostics -> {
                                    null
                                }
                            }
                        refreshContext?.let { rc ->
                            IconButton(
                                onClick = rc.onRefresh,
                                enabled = !rc.loading,
                            ) {
                                if (rc.loading) {
                                    CircularProgressIndicator(
                                        modifier = Modifier.size(20.dp),
                                        strokeWidth = 2.dp,
                                        color = MaterialTheme.colorScheme.onPrimaryContainer,
                                    )
                                } else {
                                    Icon(
                                        Icons.Default.Refresh,
                                        contentDescription = stringResource(R.string.action_refresh_apps),
                                        tint = MaterialTheme.colorScheme.onPrimaryContainer,
                                    )
                                }
                            }
                        }
                        if (currentTab == Tab.Protection) {
                            IconButton(onClick = { searchActive = true }) {
                                Icon(
                                    Icons.Default.Search,
                                    contentDescription = null,
                                    tint = MaterialTheme.colorScheme.onPrimaryContainer,
                                )
                            }
                            Box {
                                val anyFilterActive = showSystem || showRussianOnly
                                // Active-filter indicator: the old `tint = primary`
                                // did not contrast reliably against the topbar's
                                // `primaryContainer` on Material You palettes where
                                // primary and primaryContainer end up close in tone.
                                // FilledIconButton paints itself with `primary` /
                                // `onPrimary`, a pair M3 guarantees to contrast,
                                // so the indicator reads on any dynamic theme.
                                if (anyFilterActive) {
                                    FilledIconButton(onClick = { showFilterMenu = true }) {
                                        Icon(
                                            Icons.Default.FilterList,
                                            contentDescription = null,
                                        )
                                    }
                                } else {
                                    IconButton(onClick = { showFilterMenu = true }) {
                                        Icon(
                                            Icons.Default.FilterList,
                                            contentDescription = null,
                                            tint = MaterialTheme.colorScheme.onPrimaryContainer,
                                        )
                                    }
                                }
                                DropdownMenu(
                                    expanded = showFilterMenu,
                                    onDismissRequest = { showFilterMenu = false },
                                ) {
                                    DropdownMenuItem(
                                        text = { Text(stringResource(R.string.filter_show_system)) },
                                        onClick = { showSystem = !showSystem },
                                        leadingIcon = {
                                            Checkbox(
                                                checked = showSystem,
                                                onCheckedChange = null,
                                            )
                                        },
                                    )
                                    DropdownMenuItem(
                                        text = { Text(stringResource(R.string.filter_russian_only)) },
                                        onClick = { showRussianOnly = !showRussianOnly },
                                        leadingIcon = {
                                            Checkbox(
                                                checked = showRussianOnly,
                                                onCheckedChange = null,
                                            )
                                        },
                                    )
                                }
                            }
                        }
                    },
                )
            }
        },
        bottomBar = {
            NavigationBar {
                NavigationBarItem(
                    selected = currentTab == Tab.Dashboard,
                    onClick = { currentTab = Tab.Dashboard },
                    icon = { Icon(Icons.Default.Home, contentDescription = null) },
                    label = { Text(stringResource(R.string.tab_dashboard)) },
                )
                NavigationBarItem(
                    selected = currentTab == Tab.Protection,
                    onClick = { currentTab = Tab.Protection },
                    icon = { Icon(Icons.AutoMirrored.Filled.List, contentDescription = null) },
                    label = { Text(stringResource(R.string.tab_protection)) },
                )
                NavigationBarItem(
                    selected = currentTab == Tab.Diagnostics,
                    onClick = { currentTab = Tab.Diagnostics },
                    icon = { Icon(Icons.Default.CheckCircle, contentDescription = null) },
                    label = { Text(stringResource(R.string.tab_diagnostics)) },
                )
            }
        },
    ) { innerPadding ->
        val restart = selfNeedsRestart
        if (restart == null) {
            Box(
                modifier = Modifier.fillMaxSize().padding(innerPadding),
                contentAlignment = Alignment.Center,
            ) {
                CircularProgressIndicator()
            }
        } else {
            when (currentTab) {
                Tab.Dashboard -> {
                    DashboardScreen(
                        selfNeedsRestart = restart,
                        modifier = Modifier.padding(innerPadding),
                    )
                }

                Tab.Protection -> {
                    ProtectionScreen(
                        searchQuery = searchQuery,
                        showSystem = showSystem,
                        showRussianOnly = showRussianOnly,
                        modifier = Modifier.padding(innerPadding),
                    )
                }

                Tab.Diagnostics -> {
                    DiagnosticsScreen(
                        selfNeedsRestart = restart,
                        modifier = Modifier.padding(innerPadding),
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RootDeniedScreen() {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.app_name)) },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = MaterialTheme.colorScheme.errorContainer,
                        titleContentColor = MaterialTheme.colorScheme.onErrorContainer,
                    ),
            )
        },
    ) { innerPadding ->
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(innerPadding)
                    .padding(32.dp),
            contentAlignment = Alignment.Center,
        ) {
            Card(
                colors =
                    CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.errorContainer,
                    ),
            ) {
                Column(
                    modifier = Modifier.padding(24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(
                        text = stringResource(R.string.root_error_title),
                        style = MaterialTheme.typography.headlineSmall,
                        fontWeight = FontWeight.Bold,
                        color = MaterialTheme.colorScheme.onErrorContainer,
                    )
                    Spacer(Modifier.height(16.dp))
                    Text(
                        text = stringResource(R.string.root_error_message),
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onErrorContainer,
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }
    }
}
