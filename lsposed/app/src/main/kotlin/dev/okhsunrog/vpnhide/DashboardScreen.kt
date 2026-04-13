package dev.okhsunrog.vpnhide

import android.content.Intent
import android.content.pm.PackageManager
import android.database.sqlite.SQLiteDatabase
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.Uri
import android.os.Build
import android.util.Log
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.BuildConfig
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

// ── Domain types — invalid states are unrepresentable ────────────────────

sealed interface ModuleState {
    data object NotInstalled : ModuleState

    data class Installed(
        val version: String?,
        val active: Boolean,
        val targetCount: Int,
    ) : ModuleState
}

sealed interface LsposedState {
    data object NotInstalled : LsposedState

    data class InstalledInactive(
        val version: String?,
    ) : LsposedState

    data class NeedsReboot(
        val version: String?,
    ) : LsposedState

    data class Active(
        val version: String?,
        val targetCount: Int,
    ) : LsposedState
}

sealed interface ProtectionCheck {
    data object NoVpn : ProtectionCheck

    data object NeedsRestart : ProtectionCheck

    data class Checked(
        val native: NativeResult,
        val java: JavaResult,
    ) : ProtectionCheck
}

sealed interface NativeResult {
    data object Ok : NativeResult

    data class Fail(
        val passed: Int,
        val failed: Int,
    ) : NativeResult

    data object NoModule : NativeResult
}

sealed interface JavaResult {
    data object Ok : JavaResult

    data class Fail(
        val failedChecks: Int,
    ) : JavaResult

    data object HooksInactive : JavaResult
}

private enum class NativeModuleKind { Kmod, Zygisk }

private sealed interface LsposedRuntime {
    data object Inactive : LsposedRuntime

    data class Active(
        val version: String?,
    ) : LsposedRuntime
}

private sealed interface LsposedFramework {
    data object NotInstalled : LsposedFramework

    data class Installed(
        val disabled: Boolean,
    ) : LsposedFramework
}

private sealed interface LsposedConfig {
    data object ModuleNotConfigured : LsposedConfig

    data object Disabled : LsposedConfig

    data class Enabled(
        val entries: List<String>,
        val hasSystemFramework: Boolean,
        val extraEntries: List<String>,
    ) : LsposedConfig
}

private data class DashboardState(
    val kmod: ModuleState,
    val zygisk: ModuleState,
    val lsposed: LsposedState,
    val nativeInstallRecommendation: NativeInstallRecommendation?,
    val protection: ProtectionCheck,
    val issues: List<String>,
)

private data class NativeInstallRecommendation(
    val androidVersion: String,
    val kernelVersion: String,
    val kernelBranch: String?,
    val recommendedArtifact: String,
    val preferKmod: Boolean,
)

// ── Screen ───────────────────────────────────────────────────────────────

@Composable
fun DashboardScreen(
    selfNeedsRestart: Boolean,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val cm = context.getSystemService(ConnectivityManager::class.java)

    var state by remember { mutableStateOf<DashboardState?>(null) }
    var updateInfo by remember { mutableStateOf<UpdateInfo?>(null) }
    var showChangelog by remember { mutableStateOf(false) }
    var changelogData by remember { mutableStateOf<ChangelogData?>(null) }

    LaunchedEffect(Unit) {
        state = withContext(Dispatchers.IO) { loadDashboardState(cm, context, selfNeedsRestart) }
    }
    LaunchedEffect(Unit) {
        updateInfo = withContext(Dispatchers.IO) { checkForUpdate(BuildConfig.VERSION_NAME) }
    }
    LaunchedEffect(Unit) {
        if (shouldShowChangelog(context)) {
            val data = withContext(Dispatchers.IO) { loadChangelog(context) }
            if (data != null) {
                changelogData = data
                showChangelog = true
            }
            markChangelogSeen(context)
        }
    }

    if (showChangelog && changelogData != null) {
        ChangelogDialog(
            data = changelogData!!,
            onDismiss = { showChangelog = false },
        )
    }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
    ) {
        Spacer(Modifier.height(12.dp))

        val s = state
        if (s == null) {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                CircularProgressIndicator()
            }
            return@Column
        }

        // Module status cards
        Text(
            text = stringResource(R.string.dashboard_modules),
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
        )
        Spacer(Modifier.height(8.dp))

        LsposedCard(s.lsposed)
        Spacer(Modifier.height(8.dp))
        ModuleCard(stringResource(R.string.dashboard_kmod), s.kmod)
        Spacer(Modifier.height(8.dp))
        ModuleCard(stringResource(R.string.dashboard_zygisk), s.zygisk)
        s.nativeInstallRecommendation?.let { recommendation ->
            Spacer(Modifier.height(8.dp))
            NativeInstallRecommendationCard(recommendation)
        }
        updateInfo?.let { info ->
            Spacer(Modifier.height(8.dp))
            UpdateAvailableCard(info)
        }

        // Protection status
        Spacer(Modifier.height(20.dp))
        Text(
            text = stringResource(R.string.dashboard_protection),
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
        )
        Spacer(Modifier.height(8.dp))

        when (val p = s.protection) {
            is ProtectionCheck.NoVpn -> {
                StatusBanner(
                    text = stringResource(R.string.dashboard_no_vpn),
                    containerColor = MaterialTheme.colorScheme.surfaceVariant,
                    contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            is ProtectionCheck.NeedsRestart -> {
                StatusBanner(
                    text = stringResource(R.string.dashboard_needs_restart),
                    containerColor = MaterialTheme.colorScheme.tertiaryContainer,
                    contentColor = MaterialTheme.colorScheme.onTertiaryContainer,
                )
            }

            is ProtectionCheck.Checked -> {
                NativeProtectionCard(p.native)
                Spacer(Modifier.height(8.dp))
                JavaProtectionCard(p.java)
            }
        }

        // Issues
        if (s.issues.isNotEmpty()) {
            Spacer(Modifier.height(20.dp))
            Text(
                text = stringResource(R.string.dashboard_issues, s.issues.size),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                color = MaterialTheme.colorScheme.error,
            )
            Spacer(Modifier.height(8.dp))
            for (issue in s.issues) {
                StatusBanner(
                    text = issue,
                    containerColor = MaterialTheme.colorScheme.errorContainer,
                    contentColor = MaterialTheme.colorScheme.onErrorContainer,
                )
                Spacer(Modifier.height(6.dp))
            }
        }

        Spacer(Modifier.height(16.dp))
    }
}

// ── UI Components ────────────────────────────────────────────────────────

@Composable
private fun ModuleCard(
    name: String,
    state: ModuleState,
) {
    val darkTheme = isSystemInDarkTheme()
    when (state) {
        is ModuleState.NotInstalled -> {
            ModuleCardShell(
                name = name,
                version = null,
                subtitle = stringResource(R.string.dashboard_not_installed),
                dotColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.4f),
                containerColor = MaterialTheme.colorScheme.surfaceVariant,
            )
        }

        is ModuleState.Installed -> {
            val active = state.active
            ModuleCardShell(
                name = name,
                version = state.version,
                subtitle =
                    if (active) {
                        stringResource(R.string.dashboard_active_targets, state.targetCount)
                    } else {
                        stringResource(R.string.dashboard_installed_inactive)
                    },
                dotColor = if (active) Color(0xFF4CAF50) else Color(0xFFFF9800),
                containerColor =
                    if (active) {
                        if (darkTheme) Color(0xFF1B5E20).copy(alpha = 0.3f) else Color(0xFFE8F5E9)
                    } else {
                        if (darkTheme) Color(0xFFE65100).copy(alpha = 0.2f) else Color(0xFFFFF3E0)
                    },
            )
        }
    }
}

@Composable
private fun LsposedCard(state: LsposedState) {
    val darkTheme = isSystemInDarkTheme()
    val moduleName = stringResource(R.string.dashboard_lsposed_module)
    when (state) {
        is LsposedState.NotInstalled -> {
            ModuleCardShell(
                name = moduleName,
                version = null,
                subtitle = stringResource(R.string.dashboard_not_installed),
                dotColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.4f),
                containerColor = MaterialTheme.colorScheme.surfaceVariant,
            )
        }

        is LsposedState.InstalledInactive -> {
            ModuleCardShell(
                name = moduleName,
                version = state.version,
                subtitle = stringResource(R.string.dashboard_installed_inactive),
                dotColor = Color(0xFFFF9800),
                containerColor = if (darkTheme) Color(0xFFE65100).copy(alpha = 0.2f) else Color(0xFFFFF3E0),
            )
        }

        is LsposedState.NeedsReboot -> {
            ModuleCardShell(
                name = moduleName,
                version = state.version,
                subtitle = stringResource(R.string.dashboard_reboot_needed),
                dotColor = Color(0xFFFF9800),
                containerColor = if (darkTheme) Color(0xFFE65100).copy(alpha = 0.2f) else Color(0xFFFFF3E0),
            )
        }

        is LsposedState.Active -> {
            val subtitle =
                stringResource(R.string.dashboard_active_targets, state.targetCount) +
                    if (state.version != null) {
                        "\n" + stringResource(R.string.dashboard_running_version, state.version)
                    } else {
                        ""
                    }
            ModuleCardShell(
                name = moduleName,
                version = state.version,
                subtitle = subtitle,
                dotColor = Color(0xFF4CAF50),
                containerColor = if (darkTheme) Color(0xFF1B5E20).copy(alpha = 0.3f) else Color(0xFFE8F5E9),
            )
        }
    }
}

@Composable
private fun ModuleCardShell(
    name: String,
    version: String?,
    subtitle: String,
    dotColor: Color,
    containerColor: Color,
) {
    Card(
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(containerColor = containerColor),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Surface(
                shape = RoundedCornerShape(50),
                color = dotColor,
                modifier = Modifier.size(12.dp),
            ) {}
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = name,
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.Bold,
                    )
                    if (version != null) {
                        Spacer(Modifier.width(8.dp))
                        Text(
                            text = version,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                Text(
                    text = subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.7f),
                )
            }
        }
    }
}

@Composable
private fun NativeInstallRecommendationCard(recommendation: NativeInstallRecommendation) {
    val darkTheme = isSystemInDarkTheme()
    val containerColor =
        if (recommendation.preferKmod) {
            if (darkTheme) Color(0xFF0D47A1).copy(alpha = 0.28f) else Color(0xFFE3F2FD)
        } else {
            if (darkTheme) Color(0xFF4E342E).copy(alpha = 0.32f) else Color(0xFFFFF3E0)
        }

    Card(
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(containerColor = containerColor),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(
                text = stringResource(R.string.dashboard_install_recommendation_title),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
            )
            Spacer(Modifier.height(6.dp))
            Text(
                text =
                    stringResource(
                        R.string.dashboard_install_recommendation_device,
                        recommendation.androidVersion,
                        recommendation.kernelVersion,
                    ),
                style = MaterialTheme.typography.bodyMedium,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text =
                    if (recommendation.preferKmod) {
                        stringResource(
                            R.string.dashboard_install_recommendation_kmod,
                            recommendation.recommendedArtifact,
                        )
                    } else {
                        stringResource(
                            R.string.dashboard_install_recommendation_zygisk,
                            recommendation.recommendedArtifact,
                        )
                    },
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
            )
            if (!recommendation.preferKmod) {
                Spacer(Modifier.height(8.dp))
                Text(
                    text = stringResource(R.string.dashboard_install_recommendation_zygisk_warning),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
                )
            }
        }
    }
}

@Composable
private fun NativeProtectionCard(result: NativeResult) {
    val darkTheme = isSystemInDarkTheme()
    val (containerColor, statusText, statusColor) =
        when (result) {
            is NativeResult.Ok -> {
                Triple(
                    if (darkTheme) Color(0xFF1B5E20).copy(alpha = 0.3f) else Color(0xFFE8F5E9),
                    stringResource(R.string.dashboard_protection_ok),
                    Color(0xFF4CAF50),
                )
            }

            is NativeResult.Fail -> {
                val text =
                    if (result.passed > 0) {
                        stringResource(R.string.dashboard_protection_partial)
                    } else {
                        stringResource(R.string.dashboard_protection_fail)
                    }
                val color = if (result.passed > 0) Color(0xFFFF9800) else Color(0xFFC62828)
                val bg =
                    if (result.passed > 0) {
                        if (darkTheme) Color(0xFFE65100).copy(alpha = 0.2f) else Color(0xFFFFF3E0)
                    } else {
                        if (darkTheme) Color(0xFFB71C1C).copy(alpha = 0.3f) else Color(0xFFFFEBEE)
                    }
                Triple(bg, text, color)
            }

            is NativeResult.NoModule -> {
                Triple(
                    MaterialTheme.colorScheme.surfaceVariant,
                    stringResource(R.string.dashboard_protection_no_module),
                    MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    ProtectionCardShell(
        label = stringResource(R.string.dashboard_native_protection),
        statusText = statusText,
        statusColor = statusColor,
        containerColor = containerColor,
    )
}

@Composable
private fun JavaProtectionCard(result: JavaResult) {
    val darkTheme = isSystemInDarkTheme()
    val (containerColor, statusText, statusColor) =
        when (result) {
            is JavaResult.Ok -> {
                Triple(
                    if (darkTheme) Color(0xFF1B5E20).copy(alpha = 0.3f) else Color(0xFFE8F5E9),
                    stringResource(R.string.dashboard_protection_ok),
                    Color(0xFF4CAF50),
                )
            }

            is JavaResult.Fail -> {
                Triple(
                    if (darkTheme) Color(0xFFB71C1C).copy(alpha = 0.3f) else Color(0xFFFFEBEE),
                    stringResource(R.string.dashboard_protection_fail),
                    Color(0xFFC62828),
                )
            }

            is JavaResult.HooksInactive -> {
                Triple(
                    MaterialTheme.colorScheme.surfaceVariant,
                    stringResource(R.string.dashboard_protection_hooks_inactive),
                    MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    ProtectionCardShell(
        label = stringResource(R.string.dashboard_java_protection),
        statusText = statusText,
        statusColor = statusColor,
        containerColor = containerColor,
    )
}

@Composable
private fun ProtectionCardShell(
    label: String,
    statusText: String,
    statusColor: Color,
    containerColor: Color,
) {
    Card(
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(containerColor = containerColor),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(16.dp).fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = label,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium,
            )
            Text(
                text = statusText,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Bold,
                color = statusColor,
            )
        }
    }
}

@Composable
private fun StatusBanner(
    text: String,
    containerColor: Color,
    contentColor: Color,
) {
    Card(
        shape = RoundedCornerShape(8.dp),
        colors = CardDefaults.cardColors(containerColor = containerColor),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(
            text = text,
            style = MaterialTheme.typography.bodyMedium,
            color = contentColor,
            modifier = Modifier.padding(12.dp),
        )
    }
}

// ── Update & Changelog ──────────────────────────────────────────────────

@Composable
private fun UpdateAvailableCard(info: UpdateInfo) {
    val context = LocalContext.current
    val darkTheme = isSystemInDarkTheme()
    Card(
        shape = RoundedCornerShape(12.dp),
        colors =
            CardDefaults.cardColors(
                containerColor = if (darkTheme) Color(0xFF0D47A1).copy(alpha = 0.28f) else Color(0xFFE3F2FD),
            ),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(16.dp).fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    text = stringResource(R.string.update_available_title),
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold,
                )
                Text(
                    text = stringResource(R.string.update_available_subtitle, info.latestVersion),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.7f),
                )
            }
            Spacer(Modifier.width(12.dp))
            Button(
                onClick = {
                    context.startActivity(
                        Intent(Intent.ACTION_VIEW, Uri.parse(info.downloadUrl)),
                    )
                },
            ) {
                Text(stringResource(R.string.update_download))
            }
        }
    }
}

@Composable
private fun ChangelogDialog(
    data: ChangelogData,
    onDismiss: () -> Unit,
) {
    val entries = remember(data) { listOf(data.current) + data.history }
    var index by remember { mutableIntStateOf(0) }
    val entry = entries[index]
    val locale =
        LocalContext.current.resources.configuration.locales[0]
            .language
    val sectionLabels =
        mapOf(
            "added" to stringResource(R.string.changelog_section_added),
            "changed" to stringResource(R.string.changelog_section_changed),
            "fixed" to stringResource(R.string.changelog_section_fixed),
            "notes" to stringResource(R.string.changelog_section_notes),
        )

    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (entries.size > 1) {
                    IconButton(
                        onClick = { index-- },
                        enabled = index > 0,
                    ) {
                        Icon(
                            Icons.AutoMirrored.Filled.KeyboardArrowLeft,
                            contentDescription = null,
                        )
                    }
                }
                Text(
                    text = stringResource(R.string.changelog_title, entry.version),
                    modifier = Modifier.weight(1f),
                )
                if (entries.size > 1) {
                    IconButton(
                        onClick = { index++ },
                        enabled = index < entries.size - 1,
                    ) {
                        Icon(
                            Icons.AutoMirrored.Filled.KeyboardArrowRight,
                            contentDescription = null,
                        )
                    }
                }
            }
        },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                for (section in entry.sections) {
                    if (section.items.isEmpty()) continue
                    Text(
                        text = sectionLabels[section.type] ?: section.type,
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.Bold,
                    )
                    for (item in section.items) {
                        val text = if (locale == "ru") item.ru else item.en
                        Text(
                            text = "\u2022 $text",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text("OK")
            }
        },
    )
}

// ── Data loading ─────────────────────────────────────────────────────────

private const val TAG = "VpnHide-Dashboard"

private fun loadDashboardState(
    cm: ConnectivityManager,
    context: android.content.Context,
    selfNeedsRestart: Boolean,
): DashboardState {
    val issues = mutableListOf<String>()
    val res = context.resources
    val selfPkg = context.packageName

    Log.i(TAG, "=== Loading dashboard state ===")

    // ── Module detection ──
    fun parseModuleProp(dir: String): Pair<Boolean, String?> {
        val (exitCode, out) = suExec("cat $dir/module.prop 2>/dev/null")
        if (exitCode != 0 || out.isBlank()) return false to null
        val version = out.lines().firstOrNull { it.startsWith("version=") }?.removePrefix("version=")
        return true to version
    }

    fun countTargets(path: String): Int {
        val (_, out) = suExec("cat $path 2>/dev/null || true")
        return out.lines().count { line ->
            val trimmed = line.trim()
            trimmed.isNotEmpty() && !trimmed.startsWith("#") && trimmed != selfPkg
        }
    }

    fun parseProps(raw: String): Map<String, String> =
        raw
            .lines()
            .mapNotNull {
                val parts = it.split("=", limit = 2)
                if (parts.size == 2) parts[0] to parts[1] else null
            }.toMap()

    fun buildModuleVersionIssue(
        kind: NativeModuleKind,
        moduleVersion: String,
        appVersion: String,
    ): String {
        val normalizedModuleVersion = normalizeVersion(moduleVersion)
        val normalizedAppVersion = normalizeVersion(appVersion)
        return when (compareSemver(normalizedModuleVersion, normalizedAppVersion)) {
            null, 0 -> {
                res.getString(
                    when (kind) {
                        NativeModuleKind.Kmod -> R.string.dashboard_issue_kmod_version_mismatch
                        NativeModuleKind.Zygisk -> R.string.dashboard_issue_zygisk_version_mismatch
                    },
                    moduleVersion,
                    appVersion,
                )
            }

            in Int.MIN_VALUE..-1 -> {
                res.getString(
                    when (kind) {
                        NativeModuleKind.Kmod -> R.string.dashboard_issue_update_kmod
                        NativeModuleKind.Zygisk -> R.string.dashboard_issue_update_zygisk
                    },
                    moduleVersion,
                    appVersion,
                )
            }

            else -> {
                res.getString(
                    when (kind) {
                        NativeModuleKind.Kmod -> R.string.dashboard_issue_update_app_for_kmod
                        NativeModuleKind.Zygisk -> R.string.dashboard_issue_update_app_for_zygisk
                    },
                    moduleVersion,
                    appVersion,
                )
            }
        }
    }

    fun androidMajorVersionLabel(): String {
        @Suppress("DEPRECATION")
        val release =
            if (Build.VERSION.SDK_INT >= 30) {
                Build.VERSION.RELEASE_OR_CODENAME
            } else {
                Build.VERSION.RELEASE
            }.substringBefore('.')
        return "Android $release"
    }

    fun parseKernelSeries(raw: String): String? = Regex("""\b(\d+\.\d+)""").find(raw)?.groupValues?.get(1)

    fun parseKernelAndroidBranch(raw: String): String? =
        Regex("""android(\d+)""")
            .find(raw)
            ?.groupValues
            ?.get(1)
            ?.let { "Android $it" }

    fun buildNativeInstallRecommendation(): NativeInstallRecommendation? {
        val (_, kernelRaw) = suExec("uname -r 2>/dev/null")
        val kernelVersion = kernelRaw.trim().ifBlank { return null }
        val kernelSeries = parseKernelSeries(kernelVersion)
        val kernelBranch = parseKernelAndroidBranch(kernelVersion)
        val artifactKeyVersion = kernelBranch ?: androidMajorVersionLabel()
        val supportedArtifact =
            when (artifactKeyVersion to kernelSeries) {
                "Android 12" to "5.10" -> "vpnhide-kmod-android12-5.10.zip"
                "Android 13" to "5.10" -> "vpnhide-kmod-android13-5.10.zip"
                "Android 13" to "5.15" -> "vpnhide-kmod-android13-5.15.zip"
                "Android 14" to "5.15" -> "vpnhide-kmod-android14-5.15.zip"
                "Android 14" to "6.1" -> "vpnhide-kmod-android14-6.1.zip"
                "Android 15" to "6.6" -> "vpnhide-kmod-android15-6.6.zip"
                "Android 16" to "6.12" -> "vpnhide-kmod-android16-6.12.zip"
                else -> null
            }

        return if (supportedArtifact != null) {
            NativeInstallRecommendation(
                androidVersion = artifactKeyVersion,
                kernelVersion = kernelVersion,
                kernelBranch = kernelBranch,
                recommendedArtifact = supportedArtifact,
                preferKmod = true,
            )
        } else {
            NativeInstallRecommendation(
                androidVersion = artifactKeyVersion,
                kernelVersion = kernelVersion,
                kernelBranch = kernelBranch,
                recommendedArtifact = "vpnhide-zygisk.zip",
                preferKmod = false,
            )
        }
    }

    fun resolveScopeEntryLabel(entry: String): String {
        if (entry == "system" || entry == "system/0") return "System Framework"

        val packageName = entry.substringBefore('/')
        val userId = entry.substringAfter('/', "")
        return try {
            val appInfo = context.packageManager.getApplicationInfo(packageName, 0)
            val appLabel =
                context.packageManager
                    .getApplicationLabel(appInfo)
                    .toString()
                    .trim()
            when {
                appLabel.isEmpty() -> packageName
                userId.isNotEmpty() && userId != "0" -> "$appLabel ($userId)"
                else -> appLabel
            }
        } catch (_: PackageManager.NameNotFoundException) {
            packageName
        }
    }

    fun readLsposedConfig(): LsposedConfig? {
        val dbCopy = File(context.cacheDir, "vpnhide_lspd_modules_config.db")
        val dbWalCopy = File(context.cacheDir, "vpnhide_lspd_modules_config.db-wal")
        val dbShmCopy = File(context.cacheDir, "vpnhide_lspd_modules_config.db-shm")
        dbCopy.delete()
        dbWalCopy.delete()
        dbShmCopy.delete()

        val dbPath = dbCopy.absolutePath
        val walPath = dbWalCopy.absolutePath
        val shmPath = dbShmCopy.absolutePath
        val sourceBase = "/data/adb/lspd/config/modules_config.db"
        val (copyExit, copyOut) =
            suExec(
                "cat $sourceBase > $dbPath && " +
                    "chmod 644 $dbPath && " +
                    "(cat $sourceBase-wal > $walPath 2>/dev/null && chmod 644 $walPath || true) && " +
                    "(cat $sourceBase-shm > $shmPath 2>/dev/null && chmod 644 $shmPath || true) && " +
                    "ls -l $dbPath $walPath $shmPath 2>/dev/null || true",
            )
        if (copyExit != 0 || !dbCopy.isFile) {
            Log.w(TAG, "failed to copy LSPosed config db for inspection: exit=$copyExit out=$copyOut")
            return null
        }
        Log.i(TAG, "lsposed db copy: ${copyOut.trim()}")

        return try {
            SQLiteDatabase.openDatabase(dbPath, null, SQLiteDatabase.OPEN_READONLY).use { db ->
                db
                    .rawQuery(
                        "SELECT mid, enabled FROM modules WHERE module_pkg_name = ?",
                        arrayOf(selfPkg),
                    ).use { moduleCursor ->
                        if (!moduleCursor.moveToFirst()) {
                            return LsposedConfig.ModuleNotConfigured
                        }

                        val mid = moduleCursor.getLong(0)
                        val enabled = moduleCursor.getInt(1) != 0
                        if (!enabled) {
                            return LsposedConfig.Disabled
                        }

                        val scopeEntries = mutableListOf<Pair<String, Int>>()
                        db
                            .rawQuery(
                                "SELECT app_pkg_name, user_id FROM scope WHERE mid = ? ORDER BY user_id, app_pkg_name",
                                arrayOf(mid.toString()),
                            ).use { scopeCursor ->
                                while (scopeCursor.moveToNext()) {
                                    scopeEntries += scopeCursor.getString(0) to scopeCursor.getInt(1)
                                }
                            }
                        val hasSystemFramework = scopeEntries.any { (pkg, userId) -> pkg == "system" && userId == 0 }
                        val renderedEntries =
                            scopeEntries.map { (pkg, userId) ->
                                if (pkg == "system" && userId == 0) {
                                    "system"
                                } else {
                                    "$pkg/$userId"
                                }
                            }
                        val extraEntries =
                            scopeEntries
                                .filterNot { (pkg, userId) ->
                                    (pkg == "system" && userId == 0) || pkg == selfPkg
                                }.map { (pkg, userId) -> "$pkg/$userId" }

                        LsposedConfig.Enabled(
                            entries = renderedEntries,
                            hasSystemFramework = hasSystemFramework,
                            extraEntries = extraEntries,
                        )
                    }
            }
        } catch (e: Exception) {
            Log.w(TAG, "failed to inspect LSPosed config db: ${e.message}")
            null
        } finally {
            dbCopy.delete()
            dbWalCopy.delete()
            dbShmCopy.delete()
        }
    }

    fun detectLsposedFramework(): LsposedFramework {
        val moduleDir = "/data/adb/modules/zygisk_vector"
        val updateDir = "/data/adb/modules_update/zygisk_vector"
        val (exitCode, out) =
            suExec(
                "if [ -f $moduleDir/module.prop ]; then " +
                    "echo installed=1; " +
                    "echo disabled=$([ -f $moduleDir/disable ] && echo 1 || echo 0); " +
                    "elif [ -f $updateDir/module.prop ]; then " +
                    "echo installed=1; " +
                    "echo disabled=$([ -f $updateDir/disable ] && echo 1 || echo 0); " +
                    "else " +
                    "echo installed=0; " +
                    "fi",
            )
        val props = parseProps(out)
        val installed = exitCode == 0 && props["installed"] == "1"
        val disabled = props["disabled"] == "1"
        val framework =
            if (installed) {
                LsposedFramework.Installed(disabled = disabled)
            } else {
                LsposedFramework.NotInstalled
            }
        Log.i(TAG, "lsposed framework: $framework (raw=$out)")
        return framework
    }

    // kmod
    val (kmodInstalled, kmodVersion) = parseModuleProp(KMOD_MODULE_DIR)
    val (_, procExists) = suExec("[ -f $PROC_TARGETS ] && echo 1 || echo 0")
    val kmodActive = kmodInstalled && procExists.trim() == "1"
    val kmodTargetCount = if (kmodInstalled) countTargets(KMOD_TARGETS) else 0
    val kmod: ModuleState =
        if (kmodInstalled) {
            ModuleState.Installed(kmodVersion, kmodActive, kmodTargetCount)
        } else {
            ModuleState.NotInstalled
        }
    Log.i(TAG, "kmod: $kmod")

    // zygisk
    val (zygiskInstalled, zygiskVersion) = parseModuleProp(ZYGISK_MODULE_DIR)
    val zygiskStatusFile = File(context.filesDir, ZYGISK_STATUS_FILE_NAME)
    val zygiskStatusRaw =
        try {
            zygiskStatusFile.takeIf { it.isFile }?.readText().orEmpty()
        } catch (e: Exception) {
            Log.w(TAG, "failed to read zygisk status heartbeat: ${e.message}")
            ""
        }
    val zygiskProps = parseProps(zygiskStatusRaw)
    val (_, currentBootId) = suExec("cat /proc/sys/kernel/random/boot_id 2>/dev/null")
    val zygiskBootId = zygiskProps["boot_id"]
    val zygiskActive = zygiskInstalled && zygiskBootId != null && zygiskBootId == currentBootId.trim()
    val zygiskTargetCount = if (zygiskInstalled) countTargets(ZYGISK_TARGETS) else 0
    val zygisk: ModuleState =
        if (zygiskInstalled) {
            ModuleState.Installed(zygiskVersion, zygiskActive, zygiskTargetCount)
        } else {
            ModuleState.NotInstalled
        }
    Log.i(TAG, "zygisk: $zygisk (heartbeatBootId=$zygiskBootId currentBootId=${currentBootId.trim()})")
    val nativeInstallRecommendation =
        if (kmod is ModuleState.NotInstalled && zygisk is ModuleState.NotInstalled) {
            buildNativeInstallRecommendation()
        } else {
            null
        }
    Log.i(TAG, "nativeInstallRecommendation=$nativeInstallRecommendation")

    // lsposed hook status
    val (_, hookStatusRaw) = suExec("cat ${HookEntry.HOOK_STATUS_FILE} 2>/dev/null || true")
    val hookProps = parseProps(hookStatusRaw)
    val hookVersion = hookProps["version"]
    val hookBootId = hookProps["boot_id"]
    val hooksActiveThisBoot = hookBootId != null && hookBootId == currentBootId.trim()
    val lsposedTargetCount = countTargets(LSPOSED_TARGETS)
    val lsposedFramework = detectLsposedFramework()
    val lsposedConfig =
        when (lsposedFramework) {
            LsposedFramework.NotInstalled -> {
                LsposedConfig.ModuleNotConfigured
            }

            is LsposedFramework.Installed -> {
                if (lsposedFramework.disabled) {
                    LsposedConfig.Disabled
                } else {
                    readLsposedConfig()
                }
            }
        }
    val lsposedRuntime: LsposedRuntime =
        if (hooksActiveThisBoot) {
            LsposedRuntime.Active(hookVersion)
        } else {
            LsposedRuntime.Inactive
        }

    val lsposed: LsposedState =
        when (lsposedRuntime) {
            is LsposedRuntime.Active -> {
                LsposedState.Active(lsposedRuntime.version, lsposedTargetCount)
            }

            LsposedRuntime.Inactive -> {
                when (lsposedConfig) {
                    null -> {
                        LsposedState.InstalledInactive(null)
                    }

                    LsposedConfig.ModuleNotConfigured -> {
                        when (lsposedFramework) {
                            LsposedFramework.NotInstalled -> LsposedState.NotInstalled
                            is LsposedFramework.Installed -> LsposedState.InstalledInactive(null)
                        }
                    }

                    LsposedConfig.Disabled -> {
                        LsposedState.InstalledInactive(null)
                    }

                    is LsposedConfig.Enabled -> {
                        if (lsposedConfig.hasSystemFramework) {
                            LsposedState.NeedsReboot(hookVersion)
                        } else {
                            LsposedState.InstalledInactive(null)
                        }
                    }
                }
            }
        }
    Log.i(
        TAG,
        "lsposed: $lsposed (hookBootId=$hookBootId currentBootId=${currentBootId.trim()} framework=$lsposedFramework runtime=$lsposedRuntime config=$lsposedConfig)",
    )

    // ── Issues ──
    val hasNative = kmod is ModuleState.Installed || zygisk is ModuleState.Installed
    if (!hasNative) {
        issues += res.getString(R.string.dashboard_issue_no_native)
    }
    if (lsposedFramework is LsposedFramework.NotInstalled) {
        issues += res.getString(R.string.dashboard_issue_lsposed_not_installed)
    }
    if (lsposed is LsposedState.NeedsReboot) {
        issues += res.getString(R.string.dashboard_issue_reboot)
    }
    when (lsposedConfig) {
        null -> {
            issues += res.getString(R.string.dashboard_issue_lsposed_config_unreadable)
        }

        LsposedConfig.ModuleNotConfigured -> {
            if (lsposedFramework is LsposedFramework.Installed) {
                issues += res.getString(R.string.dashboard_issue_lsposed_not_enabled)
            }
        }

        LsposedConfig.Disabled -> {
            issues += res.getString(R.string.dashboard_issue_lsposed_not_enabled)
        }

        is LsposedConfig.Enabled -> {
            if (!lsposedConfig.hasSystemFramework) {
                issues += res.getString(R.string.dashboard_issue_lsposed_no_system_scope)
            }
            if (lsposedConfig.extraEntries.isNotEmpty()) {
                issues +=
                    res.getString(
                        R.string.dashboard_issue_lsposed_extra_scope,
                        lsposedConfig.extraEntries.map(::resolveScopeEntryLabel).joinToString(", "),
                    )
            }
        }
    }
    val appVersion = BuildConfig.VERSION_NAME
    if (kmod is ModuleState.Installed && kmod.version != null && normalizeVersion(kmod.version) != normalizeVersion(appVersion)) {
        issues += buildModuleVersionIssue(NativeModuleKind.Kmod, kmod.version, appVersion)
    }
    if (zygisk is ModuleState.Installed && zygisk.version != null && normalizeVersion(zygisk.version) != normalizeVersion(appVersion)) {
        issues += buildModuleVersionIssue(NativeModuleKind.Zygisk, zygisk.version, appVersion)
    }
    if (lsposed is LsposedState.Active) {
        if (lsposedTargetCount == 0) {
            issues += res.getString(R.string.dashboard_issue_no_targets)
        }
        val runningVersion = lsposed.version
        if (runningVersion != null && runningVersion != appVersion) {
            Log.w(TAG, "version mismatch: running=$runningVersion app=$appVersion")
            issues += res.getString(R.string.dashboard_issue_version_mismatch, runningVersion, appVersion)
        }
    }

    // ── Protection checks ──
    val vpnActive = isVpnActiveSync()
    Log.i(TAG, "vpnActive=$vpnActive selfNeedsRestart=$selfNeedsRestart")

    val protection: ProtectionCheck =
        when {
            !vpnActive -> {
                ProtectionCheck.NoVpn
            }

            selfNeedsRestart -> {
                ProtectionCheck.NeedsRestart
            }

            else -> {
                val native =
                    if (hasNative) {
                        runNativeProtectionCheck()
                    } else {
                        NativeResult.NoModule
                    }
                Log.i(TAG, "nativeResult=$native")

                val java =
                    if (lsposed is LsposedState.Active) {
                        runJavaProtectionCheck(cm)
                    } else {
                        JavaResult.HooksInactive
                    }
                Log.i(TAG, "javaResult=$java")

                ProtectionCheck.Checked(native, java)
            }
        }

    Log.i(TAG, "protection=$protection issues=$issues")
    Log.i(TAG, "=== Dashboard state loaded ===")

    return DashboardState(
        kmod = kmod,
        zygisk = zygisk,
        lsposed = lsposed,
        nativeInstallRecommendation = nativeInstallRecommendation,
        protection = protection,
        issues = issues,
    )
}

private fun isVpnActiveSync(): Boolean {
    val (exitCode, output) = suExec("ls /sys/class/net/ 2>/dev/null")
    if (exitCode != 0) return false
    val vpnPrefixes = listOf("tun", "wg", "ppp", "tap", "ipsec", "xfrm")
    val vpnIfaces =
        output.lines().map { it.trim() }.filter { name ->
            name.isNotEmpty() && vpnPrefixes.any { name.startsWith(it) }
        }
    if (vpnIfaces.isEmpty()) {
        Log.d(TAG, "isVpnActive: no VPN interfaces found")
        return false
    }
    return vpnIfaces.any { iface ->
        val (_, state) = suExec("cat /sys/class/net/$iface/operstate 2>/dev/null")
        val up = state.trim() == "unknown" || state.trim() == "up"
        Log.d(TAG, "isVpnActive: $iface operstate=${state.trim()} up=$up")
        up
    }
}

private fun runNativeProtectionCheck(): NativeResult {
    val checks =
        listOf(
            "ioctl_flags" to { NativeChecks.checkIoctlSiocgifflags() },
            "ioctl_mtu" to { NativeChecks.checkIoctlSiocgifmtu() },
            "ioctl_conf" to { NativeChecks.checkIoctlSiocgifconf() },
            "getifaddrs" to { NativeChecks.checkGetifaddrs() },
            "netlink_getlink" to { NativeChecks.checkNetlinkGetlink() },
            "netlink_getroute" to { NativeChecks.checkNetlinkGetroute() },
            "proc_route" to { NativeChecks.checkProcNetRoute() },
            "proc_ipv6_route" to { NativeChecks.checkProcNetIpv6Route() },
            "proc_if_inet6" to { NativeChecks.checkProcNetIfInet6() },
            "proc_tcp" to { NativeChecks.checkProcNetTcp() },
            "proc_tcp6" to { NativeChecks.checkProcNetTcp6() },
            "proc_udp" to { NativeChecks.checkProcNetUdp() },
            "proc_udp6" to { NativeChecks.checkProcNetUdp6() },
            "proc_dev" to { NativeChecks.checkProcNetDev() },
            "proc_fib_trie" to { NativeChecks.checkProcNetFibTrie() },
            "sys_class_net" to { NativeChecks.checkSysClassNet() },
        )

    var passed = 0
    var failed = 0
    var skipped = 0
    for ((name, check) in checks) {
        try {
            val result = check()
            when {
                result.startsWith("NETWORK_BLOCKED:") -> {
                    skipped++
                    Log.d(TAG, "native[$name]: NETWORK_BLOCKED")
                }

                result.contains("SELinux") || result.contains("EACCES") ||
                    result.contains("Permission denied") -> {
                    skipped++
                    Log.d(TAG, "native[$name]: SELinux blocked, skipping")
                }

                result.startsWith("PASS") -> {
                    passed++
                    Log.d(TAG, "native[$name]: PASS")
                }

                else -> {
                    failed++
                    Log.w(TAG, "native[$name]: FAIL — $result")
                }
            }
        } catch (e: Exception) {
            failed++
            Log.e(TAG, "native[$name]: exception — ${e.message}")
        }
    }

    Log.i(TAG, "native protection: passed=$passed failed=$failed skipped=$skipped")
    return when {
        passed == 0 && failed == 0 -> NativeResult.Ok

        // all SELinux-blocked = nothing leaked
        failed == 0 -> NativeResult.Ok

        passed > 0 -> NativeResult.Fail(passed, failed)

        else -> NativeResult.Fail(0, failed)
    }
}

@Suppress("DEPRECATION")
private fun runJavaProtectionCheck(cm: ConnectivityManager): JavaResult {
    val net = cm.activeNetwork
    if (net == null) {
        Log.d(TAG, "java: no active network")
        return JavaResult.Ok
    }
    val caps = cm.getNetworkCapabilities(net)
    if (caps == null) {
        Log.d(TAG, "java: no capabilities")
        return JavaResult.Ok
    }

    var failed = 0

    val hasVpn = caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
    if (hasVpn) failed++
    Log.d(TAG, "java: hasTransport(VPN)=$hasVpn")

    val notVpn = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
    if (!notVpn) failed++
    Log.d(TAG, "java: hasCapability(NOT_VPN)=$notVpn")

    val info = caps.transportInfo
    val isVpnTi = info?.javaClass?.name?.contains("VpnTransportInfo") == true
    if (isVpnTi) failed++
    Log.d(TAG, "java: transportInfo=${info?.javaClass?.name} isVpn=$isVpnTi")

    val vpnNets =
        cm.allNetworks.count {
            cm.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true
        }
    if (vpnNets > 0) failed++
    Log.d(TAG, "java: allNetworks vpnCount=$vpnNets")

    val lp = cm.getLinkProperties(net)
    val ifname = lp?.interfaceName
    val vpnPrefixes = listOf("tun", "wg", "ppp", "tap", "ipsec", "xfrm")
    val vpnIfname = ifname != null && vpnPrefixes.any { ifname.startsWith(it) }
    if (vpnIfname) failed++
    Log.d(TAG, "java: linkProperties ifname=$ifname isVpn=$vpnIfname")

    Log.i(TAG, "java protection: failed=$failed")
    return if (failed == 0) JavaResult.Ok else JavaResult.Fail(failed)
}
