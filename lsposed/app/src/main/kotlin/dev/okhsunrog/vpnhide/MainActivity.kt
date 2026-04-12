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

private const val TAG = "VpnHide"

private const val KMOD_TARGETS = "/data/adb/vpnhide_kmod/targets.txt"
private const val ZYGISK_TARGETS = "/data/adb/vpnhide_zygisk/targets.txt"
private const val ZYGISK_MODULE_TARGETS = "/data/adb/modules/vpnhide_zygisk/targets.txt"
private const val PROC_TARGETS = "/proc/vpnhide_targets"
private const val SS_UIDS_FILE = "/data/system/vpnhide_uids.txt"

data class AppEntry(
    val packageName: String,
    val label: String,
    val icon: Drawable?,
    val isSystem: Boolean,
    val selected: Boolean,
)

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
private fun suExec(cmd: String): Pair<Int, String> =
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
            RootState.Granted -> AppPickerScreen()
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RootCheckingScreen() {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.toolbar_title)) },
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
                title = { Text(stringResource(R.string.toolbar_title)) },
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

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AppPickerScreen() {
    val context = LocalContext.current
    val pm = context.packageManager

    var allApps by remember { mutableStateOf<List<AppEntry>>(emptyList()) }
    var searchQuery by remember { mutableStateOf("") }
    var showSystem by remember { mutableStateOf(false) }
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
            val (_, targetsRaw) =
                suExec(
                    "cat $KMOD_TARGETS 2>/dev/null || cat $ZYGISK_TARGETS 2>/dev/null || true",
                )
            val selected =
                targetsRaw
                    .lines()
                    .map { it.trim() }
                    .filter { it.isNotEmpty() && !it.startsWith("#") }
                    .toSet()

            val installedApps = pm.getInstalledApplications(PackageManager.GET_META_DATA)
            val entries =
                installedApps
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
                        AppEntry(
                            packageName = info.packageName,
                            label = label,
                            icon = icon,
                            isSystem = isSystem,
                            selected = info.packageName in selected,
                        )
                    }.sortedWith(compareByDescending<AppEntry> { it.selected }.thenBy { it.label.lowercase() })

            allApps = entries
            loading = false
        }
    }

    val filteredApps =
        remember(allApps, searchQuery, showSystem) {
            val q = searchQuery.trim().lowercase()
            allApps.filter { app ->
                (showSystem || !app.isSystem || app.selected) &&
                    (q.isEmpty() || app.label.lowercase().contains(q) || app.packageName.lowercase().contains(q))
            }
        }

    val selectedCount = remember(allApps) { allApps.count { it.selected } }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.toolbar_title)) },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = MaterialTheme.colorScheme.primaryContainer,
                        titleContentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                    ),
            )
        },
        bottomBar = {
            Surface(tonalElevation = 3.dp) {
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .navigationBarsPadding()
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
    ) { innerPadding ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(innerPadding),
        ) {
            // Search + system toggle
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 8.dp),
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
                            onToggle = {
                                allApps =
                                    allApps.map {
                                        if (it.packageName == app.packageName) it.copy(selected = !it.selected) else it
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
            val selected = allApps.filter { it.selected }.map { it.packageName }.sorted()
            val header = context.getString(R.string.save_header_comment)
            val body =
                "$header\n" +
                    selected.joinToString("\n") +
                    if (selected.isNotEmpty()) "\n" else ""

            try {
                val (exitCode, _) = suExecAsync(buildSaveCommand(body, selected))
                if (exitCode == 0) {
                    snackMessage = context.getString(R.string.save_success, selected.size)
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

private fun buildSaveCommand(
    body: String,
    selectedPackages: List<String>,
): String {
    val b64 = android.util.Base64.encodeToString(body.toByteArray(), android.util.Base64.NO_WRAP)

    val parts = mutableListOf<String>()

    // Write to kmod targets if dir exists
    parts += "if [ -d /data/adb/vpnhide_kmod ]; then echo '$b64' | base64 -d > $KMOD_TARGETS && chmod 644 $KMOD_TARGETS; fi"

    // Write to zygisk targets if dir exists
    parts += "if [ -d /data/adb/vpnhide_zygisk ]; then echo '$b64' | base64 -d > $ZYGISK_TARGETS && chmod 644 $ZYGISK_TARGETS; fi"

    // Copy to module dir for Magisk SELinux compat
    parts += "cp $ZYGISK_TARGETS $ZYGISK_MODULE_TARGETS 2>/dev/null; true"

    // Resolve UIDs and write to /proc/vpnhide_targets + /data/system/vpnhide_uids.txt
    if (selectedPackages.isNotEmpty()) {
        val uidResolution =
            buildString {
                append("ALL_PKGS=\"\$(pm list packages -U 2>/dev/null)\"")
                append("; UIDS=\"\"")
                for (pkg in selectedPackages) {
                    append("; U=\$(echo \"\$ALL_PKGS\" | grep '^package:$pkg ' | sed 's/.*uid://')")
                    append("; if [ -n \"\$U\" ]; then if [ -z \"\$UIDS\" ]; then UIDS=\"\$U\"; else UIDS=\"\$UIDS")
                    append("\n")
                    append("\$U\"; fi; fi")
                }
                append("; if [ -n \"\$UIDS\" ]; then echo \"\$UIDS\" > $PROC_TARGETS 2>/dev/null; echo \"\$UIDS\" > $SS_UIDS_FILE")
                append("; else echo > $PROC_TARGETS 2>/dev/null; echo > $SS_UIDS_FILE; fi")
                append("; chmod 644 $SS_UIDS_FILE 2>/dev/null")
                append("; chcon u:object_r:system_data_file:s0 $SS_UIDS_FILE 2>/dev/null")
            }
        parts += uidResolution
    } else {
        parts += "echo > $PROC_TARGETS 2>/dev/null; true"
        parts += "echo > $SS_UIDS_FILE 2>/dev/null; true"
    }

    return parts.joinToString(" ; ")
}

@Composable
private fun AppRow(
    app: AppEntry,
    onToggle: () -> Unit,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onToggle)
                .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(
            checked = app.selected,
            onCheckedChange = { onToggle() },
        )
        Spacer(Modifier.width(12.dp))
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
        }
    }
}
