package dev.okhsunrog.vpnhide

import android.content.Intent
import android.net.Uri
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
fun DashboardScreen(
    selfNeedsRestart: Boolean,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    val state by DashboardCache.state.collectAsState()
    val updateInfo by UpdateCheckCache.info.collectAsState()
    var showChangelog by remember { mutableStateOf(false) }
    var changelogData by remember { mutableStateOf<ChangelogData?>(null) }

    // Both caches are reactive to tab switches without re-doing work:
    // ensureLoaded / ensureFresh are no-ops if the data is already
    // populated or an inflight job hasn't finished yet.
    LaunchedEffect(Unit) {
        DashboardCache.ensureLoaded(scope, context, selfNeedsRestart)
        UpdateCheckCache.ensureFresh(scope, BuildConfig.VERSION_NAME)
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

        // Pinned status palette — shared by the Protection status banners
        // (NeedsRestart) and the Errors / Warnings issue banners below.
        // Theme.colorScheme.{errorContainer,tertiaryContainer} get remixed
        // by Material You to whatever the wallpaper suggests, which in
        // practice landed on "lavender" and "pink" on user devices — those
        // read as "note", not "problem". Same hardcoded pairs the module-
        // status cards use for active/inactive.
        val darkTheme = isSystemInDarkTheme()
        val errorBg = if (darkTheme) Color(0xFFB71C1C).copy(alpha = 0.3f) else Color(0xFFFFEBEE)
        val errorHeader = if (darkTheme) Color(0xFFEF9A9A) else Color(0xFFC62828)
        val warningBg = if (darkTheme) Color(0xFFE65100).copy(alpha = 0.2f) else Color(0xFFFFF3E0)
        val warningHeader = if (darkTheme) Color(0xFFFFB74D) else Color(0xFFE65100)
        val onBannerColor = MaterialTheme.colorScheme.onSurface

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
        ModuleCard(stringResource(R.string.dashboard_zygisk), s.zygisk, selfNeedsRestart)
        Spacer(Modifier.height(8.dp))
        ModuleCard(stringResource(R.string.dashboard_ports), s.ports)
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
                VpnOffPrompt(
                    onRetry = {
                        // Re-read dashboard state (re-runs its own VPN
                        // + protection probes) and re-run the diag
                        // cache so both screens move to "Ready" when
                        // VPN is back.
                        DashboardCache.refresh(scope, context, selfNeedsRestart)
                        DiagnosticsCache.retry(scope, context)
                    },
                )
            }

            is ProtectionCheck.NeedsRestart -> {
                StatusBanner(
                    text = stringResource(R.string.dashboard_needs_restart),
                    containerColor = warningBg,
                    contentColor = onBannerColor,
                )
            }

            is ProtectionCheck.Checked -> {
                NativeProtectionCard(p.native)
                Spacer(Modifier.height(8.dp))
                JavaProtectionCard(p.java)
            }
        }

        // Issues — split by severity. Errors first (user attention), then
        // warnings (working-but-suboptimal). Sections hide themselves when
        // empty so the Dashboard stays short on a healthy setup. Colors
        // come from the pinned palette declared at the top of this block.
        val errors = s.issues.filter { it.severity == IssueSeverity.ERROR }
        val warnings = s.issues.filter { it.severity == IssueSeverity.WARNING }

        if (errors.isNotEmpty()) {
            Spacer(Modifier.height(20.dp))
            Text(
                text = stringResource(R.string.dashboard_issues, errors.size),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                color = errorHeader,
            )
            Spacer(Modifier.height(8.dp))
            for (issue in errors) {
                StatusBanner(
                    text = issue.text,
                    containerColor = errorBg,
                    contentColor = onBannerColor,
                )
                Spacer(Modifier.height(6.dp))
            }
        }

        if (warnings.isNotEmpty()) {
            Spacer(Modifier.height(20.dp))
            Text(
                text = stringResource(R.string.dashboard_warnings, warnings.size),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                color = warningHeader,
            )
            Spacer(Modifier.height(8.dp))
            for (issue in warnings) {
                StatusBanner(
                    text = issue.text,
                    containerColor = warningBg,
                    contentColor = onBannerColor,
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
    selfNeedsRestart: Boolean = false,
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
            val broken = state.brokenReason
            val brokenSubtitleRes =
                when (broken) {
                    KmodBrokenReason.WrongVariant -> R.string.dashboard_kmod_broken_wrong_variant
                    KmodBrokenReason.UnsupportedKernel -> R.string.dashboard_kmod_broken_unsupported_kernel
                    KmodBrokenReason.MissingKprobes -> R.string.dashboard_kmod_broken_no_kprobes
                    KmodBrokenReason.UnknownVariantInactive -> R.string.dashboard_kmod_broken_unknown_variant
                    null -> null
                }
            ModuleCardShell(
                name = name,
                version = state.version,
                subtitle =
                    when {
                        brokenSubtitleRes != null -> stringResource(brokenSubtitleRes)
                        active -> stringResource(R.string.dashboard_active_targets, state.targetCount)
                        selfNeedsRestart -> stringResource(R.string.dashboard_installed_restart_app)
                        else -> stringResource(R.string.dashboard_installed_inactive)
                    },
                dotColor =
                    when {
                        broken != null -> Color(0xFFB71C1C)
                        active -> Color(0xFF4CAF50)
                        else -> Color(0xFFFF9800)
                    },
                containerColor =
                    when {
                        broken != null -> {
                            if (darkTheme) Color(0xFFB71C1C).copy(alpha = 0.3f) else Color(0xFFFFEBEE)
                        }

                        active -> {
                            if (darkTheme) Color(0xFF1B5E20).copy(alpha = 0.3f) else Color(0xFFE8F5E9)
                        }

                        else -> {
                            if (darkTheme) Color(0xFFE65100).copy(alpha = 0.2f) else Color(0xFFFFF3E0)
                        }
                    },
            )
        }
    }
}

@Composable
private fun LsposedCard(state: LsposedState) {
    val darkTheme = isSystemInDarkTheme()
    val moduleName = stringResource(R.string.dashboard_lsposed_module)
    val installedVersion = BuildConfig.VERSION_NAME
    when (state) {
        is LsposedState.NotInstalled -> {
            ModuleCardShell(
                name = moduleName,
                version = installedVersion,
                subtitle = stringResource(R.string.dashboard_not_installed),
                dotColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.4f),
                containerColor = MaterialTheme.colorScheme.surfaceVariant,
            )
        }

        is LsposedState.InstalledInactive -> {
            ModuleCardShell(
                name = moduleName,
                version = installedVersion,
                subtitle = stringResource(R.string.dashboard_installed_inactive),
                dotColor = Color(0xFFFF9800),
                containerColor = if (darkTheme) Color(0xFFE65100).copy(alpha = 0.2f) else Color(0xFFFFF3E0),
            )
        }

        is LsposedState.NeedsReboot -> {
            ModuleCardShell(
                name = moduleName,
                version = installedVersion,
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
                version = installedVersion,
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
    val entries = remember(data) { data.history }
    if (entries.isEmpty()) {
        onDismiss()
        return
    }
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
