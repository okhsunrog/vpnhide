package dev.okhsunrog.vpnhide

import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.drawable.Drawable
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Home
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.graphics.drawable.toBitmap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

private const val TAG = "VpnHide"

internal const val KMOD_TARGETS = "/data/adb/vpnhide_kmod/targets.txt"
internal const val ZYGISK_TARGETS = "/data/adb/vpnhide_zygisk/targets.txt"
internal const val ZYGISK_MODULE_TARGETS = "/data/adb/modules/vpnhide_zygisk/targets.txt"
internal const val LSPOSED_TARGETS = "/data/adb/vpnhide_lsposed/targets.txt"
internal const val PROC_TARGETS = "/proc/vpnhide_targets"
internal const val SS_UIDS_FILE = "/data/system/vpnhide_uids.txt"
internal const val KMOD_MODULE_DIR = "/data/adb/modules/vpnhide_kmod"
internal const val ZYGISK_MODULE_DIR = "/data/adb/modules/vpnhide_zygisk"
internal const val ZYGISK_STATUS_FILE_NAME = "vpnhide_zygisk_active"

data class AppEntry(
    val packageName: String,
    val label: String,
    val icon: Drawable?,
    val isSystem: Boolean,
    val kmod: Boolean = false,
    val zygisk: Boolean = false,
    val lsposed: Boolean = false,
) {
    val anySelected get() = kmod || zygisk || lsposed
}

private sealed class RootState {
    data object Checking : RootState()

    data object Granted : RootState()

    data object Denied : RootState()
}

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent { VpnHideApp() }
    }
}

/**
 * Returns exit code and stdout. Exit code -1 means the su binary
 * couldn't be executed at all (not installed or permission denied).
 */
internal fun suExec(cmd: String): Pair<Int, String> =
    try {
        val proc = Runtime.getRuntime().exec(arrayOf("su", "-c", cmd))
        try {
            val stderrDrain = Thread { proc.errorStream.readBytes() }
            stderrDrain.start()
            val stdout = proc.inputStream.bufferedReader().readText()
            val exitCode = proc.waitFor()
            stderrDrain.join()
            exitCode to stdout
        } finally {
            proc.destroy()
        }
    } catch (e: Exception) {
        Log.e(TAG, "su exec failed: ${e.message}")
        -1 to ""
    }

private suspend fun suExecAsync(cmd: String): Pair<Int, String> = withContext(Dispatchers.IO) { suExec(cmd) }

internal fun cleanupStaleZygiskStatus(context: android.content.Context) {
    val statusFile = File(context.filesDir, ZYGISK_STATUS_FILE_NAME)
    if (!statusFile.isFile) return

    val props =
        try {
            statusFile
                .readLines()
                .mapNotNull {
                    val parts = it.split("=", limit = 2)
                    if (parts.size == 2) parts[0] to parts[1] else null
                }.toMap()
        } catch (e: Exception) {
            Log.w(TAG, "cleanupStaleZygiskStatus: failed to read heartbeat: ${e.message}")
            emptyMap()
        }

    val heartbeatBootId = props["boot_id"]
    val (_, currentBootIdRaw) = suExec("cat /proc/sys/kernel/random/boot_id 2>/dev/null")
    val currentBootId = currentBootIdRaw.trim()
    val stale =
        heartbeatBootId.isNullOrBlank() ||
            heartbeatBootId != currentBootId

    if (stale) {
        if (statusFile.delete()) {
            Log.i(
                TAG,
                "cleanupStaleZygiskStatus: deleted stale heartbeat " +
                    "(bootId=$heartbeatBootId currentBootId=$currentBootId)",
            )
        } else {
            Log.w(TAG, "cleanupStaleZygiskStatus: failed to delete stale heartbeat")
        }
    }
}

/**
 * Ensure the VPN Hide app itself is in all 3 target lists + resolve UIDs.
 * Returns true if self had to be added to any list (= hooks may not be
 * applied to the current process, restart needed for zygisk).
 * Called once at app startup; result is shared with all screens.
 */
internal fun ensureSelfInTargets(selfPkg: String): Boolean {
    var added = false

    fun addIfMissing(
        path: String,
        dirCheck: String?,
    ) {
        if (dirCheck != null) {
            val (_, exists) = suExec("[ -d $dirCheck ] && echo 1 || echo 0")
            if (exists.trim() != "1") {
                Log.d(TAG, "ensureSelfInTargets: $dirCheck not found, skipping $path")
                return
            }
        }
        val (_, raw) = suExec("cat $path 2>/dev/null || true")
        val existing = raw.lines().map { it.trim() }.filter { it.isNotEmpty() && !it.startsWith("#") }
        if (selfPkg in existing) {
            Log.d(TAG, "ensureSelfInTargets: $selfPkg already in $path")
            return
        }
        val newBody =
            "# Managed by VPN Hide app\n" +
                (existing + selfPkg).sorted().joinToString("\n") + "\n"
        val b64 = android.util.Base64.encodeToString(newBody.toByteArray(), android.util.Base64.NO_WRAP)
        suExec("echo '$b64' | base64 -d > $path && chmod 644 $path")
        Log.i(TAG, "ensureSelfInTargets: added $selfPkg to $path")
        added = true
    }

    addIfMissing(KMOD_TARGETS, "/data/adb/vpnhide_kmod")
    addIfMissing(ZYGISK_TARGETS, "/data/adb/vpnhide_zygisk")
    suExec("mkdir -p /data/adb/vpnhide_lsposed")
    addIfMissing(LSPOSED_TARGETS, null)

    // Resolve UIDs so hooks pick us up immediately (kmod + lsposed support live reload)
    val uidCmd =
        buildString {
            append("ALL_PKGS=\"\$(pm list packages -U 2>/dev/null)\"")
            append("; SELF_UID=\$(echo \"\$ALL_PKGS\" | grep '^package:$selfPkg ' | sed 's/.*uid://')")
            append("; if [ -f $PROC_TARGETS ] && [ -n \"\$SELF_UID\" ]; then")
            append("   EXISTING=\$(cat $PROC_TARGETS 2>/dev/null)")
            append(";  echo \"\$EXISTING\" | grep -q \"^\$SELF_UID\$\" || echo \"\$SELF_UID\" >> $PROC_TARGETS")
            append("; fi")
            append("; if [ -n \"\$SELF_UID\" ]; then")
            append("   EXISTING2=\$(cat $SS_UIDS_FILE 2>/dev/null)")
            append(
                ";  echo \"\$EXISTING2\" | grep -q \"^\$SELF_UID\$\" || { echo \"\$SELF_UID\" >> $SS_UIDS_FILE; chmod 644 $SS_UIDS_FILE; chcon u:object_r:system_data_file:s0 $SS_UIDS_FILE 2>/dev/null; }",
            )
            append("; fi")
        }
    suExec(uidCmd)
    Log.d(TAG, "ensureSelfInTargets: done, added=$added")
    return added
}

/** Quick root check: run `su -c id` and see if it succeeds. */
private fun checkRootAccess(): Boolean {
    val (exitCode, stdout) = suExec("id")
    return exitCode == 0 && stdout.contains("uid=0")
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun VpnHideApp() {
    val darkTheme = isSystemInDarkTheme()
    val colorScheme =
        if (android.os.Build.VERSION.SDK_INT >= 31) {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        } else {
            if (darkTheme) darkColorScheme() else lightColorScheme()
        }

    MaterialTheme(colorScheme = colorScheme) {
        var rootState by remember { mutableStateOf<RootState>(RootState.Checking) }

        LaunchedEffect(Unit) {
            rootState =
                withContext(Dispatchers.IO) {
                    if (checkRootAccess()) RootState.Granted else RootState.Denied
                }
        }

        when (rootState) {
            RootState.Checking -> RootCheckingScreen()
            RootState.Denied -> RootDeniedScreen()
            RootState.Granted -> MainScreen()
        }
    }
}

private enum class Tab { Dashboard, Apps, Diagnostics }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MainScreen() {
    val context = LocalContext.current
    var currentTab by remember { mutableStateOf(Tab.Dashboard) }
    var selfNeedsRestart by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        selfNeedsRestart =
            withContext(Dispatchers.IO) {
                cleanupStaleZygiskStatus(context)
                ensureSelfInTargets(context.packageName)
            }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.app_name)) },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = MaterialTheme.colorScheme.primaryContainer,
                        titleContentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                    ),
            )
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
                    selected = currentTab == Tab.Apps,
                    onClick = { currentTab = Tab.Apps },
                    icon = { Icon(Icons.AutoMirrored.Filled.List, contentDescription = null) },
                    label = { Text(stringResource(R.string.tab_apps)) },
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
        when (currentTab) {
            Tab.Dashboard -> {
                DashboardScreen(
                    selfNeedsRestart = selfNeedsRestart,
                    modifier = Modifier.padding(innerPadding),
                )
            }

            Tab.Apps -> {
                AppPickerScreen(
                    modifier = Modifier.padding(innerPadding),
                )
            }

            Tab.Diagnostics -> {
                DiagnosticsScreen(
                    selfNeedsRestart = selfNeedsRestart,
                    modifier = Modifier.padding(innerPadding),
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RootCheckingScreen() {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.app_name)) },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = MaterialTheme.colorScheme.primaryContainer,
                        titleContentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                    ),
            )
        },
    ) { innerPadding ->
        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(innerPadding),
            contentAlignment = Alignment.Center,
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                CircularProgressIndicator()
                Spacer(Modifier.height(16.dp))
                Text(
                    text = stringResource(R.string.root_checking),
                    style = MaterialTheme.typography.bodyLarge,
                )
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

/** Which installed modules are present (detected once at load). */
data class InstalledModules(
    val kmod: Boolean = false,
    val zygisk: Boolean = false,
)

@Composable
fun AppPickerScreen(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val pm = context.packageManager

    var allApps by remember { mutableStateOf<List<AppEntry>>(emptyList()) }
    var installed by remember { mutableStateOf(InstalledModules()) }
    var showSystem by remember { mutableStateOf(false) }
    var searchQuery by remember { mutableStateOf("") }
    var loading by remember { mutableStateOf(true) }
    var saving by remember { mutableStateOf(false) }
    var dirty by remember { mutableStateOf(false) }
    var snackMessage by remember { mutableStateOf<String?>(null) }
    val snackbarHostState = remember { SnackbarHostState() }

    LaunchedEffect(snackMessage) {
        snackMessage?.let {
            snackbarHostState.showSnackbar(it)
            snackMessage = null
        }
    }

    LaunchedEffect(Unit) {
        withContext(Dispatchers.IO) {
            // Detect which native modules are installed
            val (_, detectOut) =
                suExec(
                    "echo kmod=\$([ -d $KMOD_MODULE_DIR ] && echo 1 || echo 0);" +
                        "echo zygisk=\$([ -d $ZYGISK_MODULE_DIR ] && echo 1 || echo 0)",
                )
            val flags =
                detectOut
                    .lines()
                    .filter { it.contains("=") }
                    .associate {
                        val (k, v) = it.split("=", limit = 2)
                        k to (v == "1")
                    }
            installed =
                InstalledModules(
                    kmod = flags["kmod"] == true,
                    zygisk = flags["zygisk"] == true,
                )

            // Read 3 separate target lists
            fun readTargets(path: String): Set<String> {
                val (_, raw) = suExec("cat $path 2>/dev/null || true")
                return raw
                    .lines()
                    .map { it.trim() }
                    .filter { it.isNotEmpty() && !it.startsWith("#") }
                    .toSet()
            }
            val kmodTargets = readTargets(KMOD_TARGETS)
            val zygiskTargets = readTargets(ZYGISK_TARGETS)
            val lsposedTargets = readTargets(LSPOSED_TARGETS)

            val selfPkg = context.packageName
            val installedApps = pm.getInstalledApplications(PackageManager.GET_META_DATA)
            val entries =
                installedApps
                    .filter { it.packageName != selfPkg }
                    .map { info ->
                        val label =
                            try {
                                pm.getApplicationLabel(info).toString()
                            } catch (_: Exception) {
                                info.packageName
                            }
                        val icon =
                            try {
                                pm.getApplicationIcon(info)
                            } catch (_: Exception) {
                                null
                            }
                        val isSystem = (info.flags and ApplicationInfo.FLAG_SYSTEM) != 0
                        val pkg = info.packageName
                        AppEntry(
                            packageName = pkg,
                            label = label,
                            icon = icon,
                            isSystem = isSystem,
                            kmod = pkg in kmodTargets,
                            zygisk = pkg in zygiskTargets,
                            lsposed = pkg in lsposedTargets,
                        )
                    }.sortedWith(compareByDescending<AppEntry> { it.anySelected }.thenBy { it.label.lowercase() })

            allApps = entries
            loading = false
        }
    }

    val filteredApps =
        remember(allApps, searchQuery, showSystem) {
            val q = searchQuery.trim().lowercase()
            allApps.filter { app ->
                (showSystem || !app.isSystem || app.anySelected) &&
                    (q.isEmpty() || app.label.lowercase().contains(q) || app.packageName.lowercase().contains(q))
            }
        }

    val selectedCount = remember(allApps) { allApps.count { it.anySelected } }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbarHostState) },
        bottomBar = {
            Surface(tonalElevation = 3.dp) {
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = stringResource(R.string.selected_count, selectedCount),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f),
                    )
                    Button(
                        onClick = {
                            saving = true
                            dirty = false
                        },
                        enabled = dirty && !saving,
                    ) {
                        Text(stringResource(R.string.btn_save))
                    }
                }
            }
        },
        modifier = modifier,
    ) { innerPadding ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(innerPadding),
        ) {
            // Hint card
            Card(
                shape = RoundedCornerShape(12.dp),
                colors =
                    CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.secondaryContainer,
                    ),
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp)
                        .padding(top = 4.dp),
            ) {
                Column(modifier = Modifier.padding(12.dp)) {
                    Text(
                        text = stringResource(R.string.apps_hint_toggles),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSecondaryContainer,
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        text = stringResource(R.string.apps_hint_zygisk),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSecondaryContainer.copy(alpha = 0.8f),
                    )
                }
            }

            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedTextField(
                    value = searchQuery,
                    onValueChange = { searchQuery = it },
                    placeholder = { Text(stringResource(R.string.search_placeholder)) },
                    singleLine = true,
                    modifier = Modifier.weight(1f),
                )
                Spacer(Modifier.width(8.dp))
                FilterChip(
                    selected = showSystem,
                    onClick = { showSystem = !showSystem },
                    label = { Text(stringResource(R.string.filter_system)) },
                )
            }

            if (loading) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center,
                ) {
                    CircularProgressIndicator()
                }
            } else {
                LazyColumn(modifier = Modifier.fillMaxSize()) {
                    items(filteredApps, key = { it.packageName }) { app ->
                        AppRow(
                            app = app,
                            installed = installed,
                            onToggle = { layer ->
                                allApps =
                                    allApps.map {
                                        if (it.packageName != app.packageName) {
                                            it
                                        } else {
                                            when (layer) {
                                                Layer.KMOD -> it.copy(kmod = !it.kmod)
                                                Layer.ZYGISK -> it.copy(zygisk = !it.zygisk)
                                                Layer.LSPOSED -> it.copy(lsposed = !it.lsposed)
                                            }
                                        }
                                    }
                                dirty = true
                            },
                            onToggleAll = {
                                allApps =
                                    allApps.map {
                                        if (it.packageName != app.packageName) {
                                            it
                                        } else {
                                            val newState = !it.anySelected
                                            it.copy(
                                                kmod = if (installed.kmod) newState else false,
                                                zygisk = if (installed.zygisk) newState else false,
                                                lsposed = newState,
                                            )
                                        }
                                    }
                                dirty = true
                            },
                        )
                    }
                }
            }
        }
    }

    // Save effect
    if (saving) {
        LaunchedEffect(Unit) {
            // Always include self in all lists (self is hidden from UI but must stay in targets)
            val selfPkg = context.packageName
            val kmodPkgs = (allApps.filter { it.kmod }.map { it.packageName } + selfPkg).distinct().sorted()
            val zygiskPkgs = (allApps.filter { it.zygisk }.map { it.packageName } + selfPkg).distinct().sorted()
            val lsposedPkgs = (allApps.filter { it.lsposed }.map { it.packageName } + selfPkg).distinct().sorted()
            val header = context.getString(R.string.save_header_comment)

            try {
                val (exitCode, _) =
                    suExecAsync(
                        buildSaveCommand(header, kmodPkgs, zygiskPkgs, lsposedPkgs),
                    )
                val totalSelected = allApps.count { it.anySelected }
                if (exitCode == 0) {
                    snackMessage = context.getString(R.string.save_success, totalSelected)
                } else if (exitCode == -1) {
                    snackMessage = context.getString(R.string.save_failed_root)
                    dirty = true
                } else {
                    snackMessage = context.getString(R.string.save_failed_exit, exitCode)
                    dirty = true
                }
            } catch (e: Exception) {
                snackMessage = context.getString(R.string.save_failed_error, e.message ?: "")
                dirty = true
            }
            saving = false
        }
    }
}

internal enum class Layer { KMOD, ZYGISK, LSPOSED }

private fun buildSaveCommand(
    header: String,
    kmodPkgs: List<String>,
    zygiskPkgs: List<String>,
    lsposedPkgs: List<String>,
): String {
    fun encodeBody(pkgs: List<String>): String {
        val body = "$header\n" + pkgs.joinToString("\n") + if (pkgs.isNotEmpty()) "\n" else ""
        return android.util.Base64.encodeToString(body.toByteArray(), android.util.Base64.NO_WRAP)
    }

    val parts = mutableListOf<String>()

    // Write kmod targets
    val kmodB64 = encodeBody(kmodPkgs)
    parts += "if [ -d /data/adb/vpnhide_kmod ]; then echo '$kmodB64' | base64 -d > $KMOD_TARGETS && chmod 644 $KMOD_TARGETS; fi"

    // Write zygisk targets
    val zygiskB64 = encodeBody(zygiskPkgs)
    parts += "if [ -d /data/adb/vpnhide_zygisk ]; then echo '$zygiskB64' | base64 -d > $ZYGISK_TARGETS && chmod 644 $ZYGISK_TARGETS; fi"
    parts += "cp $ZYGISK_TARGETS $ZYGISK_MODULE_TARGETS 2>/dev/null; true"

    // Write lsposed targets (always — the dir is created by service.sh or us)
    val lsposedB64 = encodeBody(lsposedPkgs)
    parts += "mkdir -p /data/adb/vpnhide_lsposed"
    parts += "echo '$lsposedB64' | base64 -d > $LSPOSED_TARGETS && chmod 644 $LSPOSED_TARGETS"

    // Resolve kmod UIDs → /proc/vpnhide_targets
    if (kmodPkgs.isNotEmpty()) {
        parts += buildUidResolver(kmodPkgs, PROC_TARGETS)
    } else {
        parts += "echo > $PROC_TARGETS 2>/dev/null; true"
    }

    // Resolve lsposed UIDs → /data/system/vpnhide_uids.txt
    if (lsposedPkgs.isNotEmpty()) {
        parts += buildUidResolver(lsposedPkgs, SS_UIDS_FILE)
        parts += "chmod 644 $SS_UIDS_FILE 2>/dev/null"
        parts += "chcon u:object_r:system_data_file:s0 $SS_UIDS_FILE 2>/dev/null"
    } else {
        parts += "echo > $SS_UIDS_FILE 2>/dev/null; true"
    }

    return parts.joinToString(" ; ")
}

private fun buildUidResolver(
    packages: List<String>,
    outputFile: String,
): String =
    buildString {
        append("ALL_PKGS=\"\$(pm list packages -U 2>/dev/null)\"")
        append("; UIDS=\"\"")
        for (pkg in packages) {
            append("; U=\$(echo \"\$ALL_PKGS\" | grep '^package:$pkg ' | sed 's/.*uid://')")
            append("; if [ -n \"\$U\" ]; then if [ -z \"\$UIDS\" ]; then UIDS=\"\$U\"; else UIDS=\"\$UIDS")
            append("\n")
            append("\$U\"; fi; fi")
        }
        append("; if [ -n \"\$UIDS\" ]; then echo \"\$UIDS\" > $outputFile 2>/dev/null")
        append("; else echo > $outputFile 2>/dev/null; fi")
    }

@Composable
private fun AppRow(
    app: AppEntry,
    installed: InstalledModules,
    onToggle: (Layer) -> Unit,
    onToggleAll: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onToggleAll)
                .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        app.icon?.let { drawable ->
            Image(
                bitmap = drawable.toBitmap(48, 48).asImageBitmap(),
                contentDescription = null,
                modifier = Modifier.size(40.dp),
            )
            Spacer(Modifier.width(12.dp))
        }
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = app.label,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium,
            )
            Text(
                text = app.packageName,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(4.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                LayerChip("L", app.lsposed, true) { onToggle(Layer.LSPOSED) }
                if (installed.kmod) {
                    LayerChip("K", app.kmod, true) { onToggle(Layer.KMOD) }
                }
                if (installed.zygisk) {
                    LayerChip("Z", app.zygisk, true) { onToggle(Layer.ZYGISK) }
                }
            }
        }
    }
}

@Composable
private fun LayerChip(
    label: String,
    enabled: Boolean,
    available: Boolean,
    onClick: () -> Unit,
) {
    val containerColor = if (enabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant
    val contentColor = if (enabled) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant
    Surface(
        shape = RoundedCornerShape(4.dp),
        color = containerColor,
        modifier = Modifier.clickable(enabled = available, onClick = onClick),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            fontWeight = FontWeight.Bold,
            color = contentColor,
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
        )
    }
}
