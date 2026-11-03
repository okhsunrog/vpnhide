package dev.okhsunrog.vpnhide

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.HelpOutline
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Shield
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.okhsunrog.vpnhide.diagnostics.retryDiagnosticsAndDashboard
import dev.okhsunrog.vpnhide.maintenance.CoreMaintenanceCard
import dev.okhsunrog.vpnhide.migration.LegacyImportPrompt
import dev.okhsunrog.vpnhide.settings.LocalSettingsInteractor
import dev.okhsunrog.vpnhide.settings.LocalSettingsState
import dev.okhsunrog.vpnhide.settings.SettingsRepository
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard
import dev.okhsunrog.vpnhide.ui.components.EnhancedOutlinedButton
import dev.okhsunrog.vpnhide.ui.components.GroupedCard
import dev.okhsunrog.vpnhide.ui.components.IconBubble
import dev.okhsunrog.vpnhide.ui.components.MetricContent
import dev.okhsunrog.vpnhide.ui.components.MetricGrid
import dev.okhsunrog.vpnhide.ui.components.ModuleHeading
import dev.okhsunrog.vpnhide.ui.components.SectionHeader
import dev.okhsunrog.vpnhide.ui.components.container
import dev.okhsunrog.vpnhide.ui.theme.AppColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import dev.okhsunrog.vpnhide.ui.components.SectionHeader as SharedSectionHeader

@Composable
fun DashboardScreen(
    selfNeedsRestart: Boolean,
    onOpenDiagnostics: () -> Unit,
    modifier: Modifier = Modifier,
    onOpenHelp: (String?) -> Unit = {},
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    val display by DashboardCache.presentation.collectAsStateWithLifecycle()
    val state = display.state
    val loadError by DashboardCache.error.collectAsStateWithLifecycle()
    val updateInfo by UpdateCheckCache.info.collectAsStateWithLifecycle()
    // What the user is looking at, classified once from the shared diagnostic
    // projection (routing knowledge, the run in flight, the latest attempt, the
    // measurement and its staleness). The hero and the prompt under it are two
    // wordings of it (heroVisual), never an overlay of flows of different vintages.
    var showChangelog by remember { mutableStateOf(false) }
    var changelogData by remember { mutableStateOf<ChangelogData?>(null) }
    var showContact by remember { mutableStateOf(false) }
    // Captured when the banner is tapped, not read live: a successful import
    // refreshes the dashboard (prompt → null) while the dialog is still showing
    // its result.
    var legacyImportDialog by remember { mutableStateOf<LegacyImportPrompt?>(null) }

    if (showContact) {
        ContactModal(onDismiss = { showContact = false })
    }
    legacyImportDialog?.let { prompt ->
        LegacyImportDialog(prompt = prompt, onDismiss = { legacyImportDialog = null })
    }

    // Both caches are reactive to tab switches without re-doing work:
    // ensureLoaded / ensureFresh are no-ops if the data is already
    // populated or an inflight job hasn't finished yet.
    LaunchedEffect(state == null && loadError == null, selfNeedsRestart) {
        if (state == null && loadError == null) {
            DashboardCache.ensureLoaded(context, selfNeedsRestart)
        }
        UpdateCheckCache.ensureFresh(scope, BuildConfig.VERSION_NAME)
    }
    LaunchedEffect(Unit) {
        // Read the persisted flag directly rather than LocalSettingsState: the
        // ambient snapshot is the default (false) until DataStore loads, which
        // would race this cold-start effect and pop the changelog anyway. When
        // suppressed we also skip markChangelogSeen, so turning the toggle back
        // off still shows the changelog for the current version.
        val suppress =
            SettingsRepository(context.applicationContext).settings.first().suppressChangelog
        if (!suppress && shouldShowChangelog(context)) {
            val data = withContext(Dispatchers.IO) { loadChangelog(context) }
            // Only raise the dialog when there's something to show — the
            // emptiness guard lives here (a side-effect scope) rather than
            // inside ChangelogDialog's composition body.
            if (data != null && data.history.isNotEmpty()) {
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
        dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticStatusCard(display.diagnostics) {
            retryDiagnosticsAndDashboard(context, selfNeedsRestart)
        }
        Spacer(Modifier.height(12.dp))
        CoreMaintenanceCard()
        NativeCoreStatusCard(
            display.diagnostics.sources.core
                .nativeStatusSnapshot(),
        )
        Spacer(Modifier.height(12.dp))

        // Pinned status palette — shared by the Protection status banners
        // (NeedsRestart) and the Errors / Warnings issue banners below.
        // Theme.colorScheme.{errorContainer,tertiaryContainer} get remixed
        // by Material You to whatever the wallpaper suggests, which in
        // practice landed on "lavender" and "pink" on user devices — those
        // read as "note", not "problem". Same hardcoded pairs the module-
        // status cards use for active/inactive.
        val errorBg = StatusColors.errorContainer()
        val errorHeader = StatusColors.errorHeader()
        val warningBg = StatusColors.warningContainer()
        val warningHeader = StatusColors.warningHeader()
        val infoBg = StatusColors.neutralContainer()
        val infoHeader = StatusColors.neutralHeader()
        val onBannerColor = MaterialTheme.colorScheme.onSurface

        val s = state
        val error = loadError
        if (error != null) {
            DashboardLoadErrorCard(
                title = stringResource(R.string.dashboard_load_failed_title),
                message =
                    stringResource(
                        if (s == null) {
                            R.string.dashboard_load_failed_message
                        } else {
                            R.string.dashboard_refresh_failed_message
                        },
                    ),
                containerColor = errorBg,
                titleColor = errorHeader,
                contentColor = onBannerColor,
                onRetry = { DashboardCache.refresh(context, selfNeedsRestart) },
            )
            Spacer(Modifier.height(12.dp))
            if (s == null) {
                CoreComponentCard(display.core, index = 0, count = 1)
                return@Column
            }
        }
        if (s == null) {
            CoreComponentCard(display.core, index = 0, count = 1)
            return@Column
        }
        if (!display.diagnostics.rootJoined) {
            CoreComponentCard(display.core, index = 0, count = 1)
            Spacer(Modifier.height(12.dp))
            Text(stringResource(R.string.autonomous_metadata_pending))
            EnhancedButton(onClick = { DashboardCache.refresh(context, selfNeedsRestart) }) {
                Text(stringResource(R.string.retry))
            }
            return@Column
        }

        val loadedState = s

        // Messages split by severity. Only errors/warnings affect the hero:
        // info messages are neutral notes rendered below without changing the
        // overall "Protected" state or issue count.
        val errors = loadedState.messages.filter { it.severity == DashboardMessageSeverity.ERROR }
        val warnings = loadedState.messages.filter { it.severity == DashboardMessageSeverity.WARNING }
        val infos = loadedState.messages.filter { it.severity == DashboardMessageSeverity.INFO }

        Spacer(Modifier.height(20.dp))

        // Module status cards — one grouped block (byIndex corners).
        SectionHeader(stringResource(R.string.dashboard_modules))
        Spacer(Modifier.height(8.dp))
        // Three components; Ports is a subordinate Core function.
        Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
            JavaBackendCard(loadedState.lsposed, index = 0, count = 2)
            CoreComponentCard(loadedState.core, index = 1, count = 2)
        }
        updateInfo?.let { info ->
            Spacer(Modifier.height(8.dp))
            UpdateAvailableCard(info)
        }

        // A pre-1.0 config is still on disk and this install already has roles,
        // so the startup importer left the call to the user.
        val legacyPrompt = loadedState.legacyImport
        if (legacyPrompt != null && !LocalSettingsState.current.legacyImportDismissed) {
            val interactor = LocalSettingsInteractor.current
            Spacer(Modifier.height(8.dp))
            LegacyImportBanner(
                prompt = legacyPrompt,
                containerColor = infoBg,
                contentColor = onBannerColor,
                onImport = { legacyImportDialog = legacyPrompt },
                onHide = { interactor.setLegacyImportDismissed(true) },
            )
        }

        val onContact = { showContact = true }
        MessageSection(errors, R.string.dashboard_issues, errorHeader, errorBg, onBannerColor, onOpenDiagnostics, onContact, onOpenHelp)
        MessageSection(
            warnings,
            R.string.dashboard_warnings,
            warningHeader,
            warningBg,
            onBannerColor,
            onOpenDiagnostics,
            onContact,
            onOpenHelp,
        )
        MessageSection(infos, R.string.dashboard_info, infoHeader, infoBg, onBannerColor, onOpenDiagnostics, onContact, onOpenHelp)

        Spacer(Modifier.height(20.dp))
        HelpGuideEntry(onClick = { onOpenHelp(null) })

        Spacer(Modifier.height(16.dp))
    }
}

// A quiet, always-present entry into the offline guide's table of contents — the
// contextual entries on issue banners deep-link into specific articles; this is
// the general "open the whole guide" affordance.
@Composable
private fun HelpGuideEntry(onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        color = AppColors.cardContainer,
        shape = RoundedCornerShape(16.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.AutoMirrored.Filled.HelpOutline,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
            )
            Spacer(Modifier.width(12.dp))
            Text(
                text = stringResource(R.string.help_title),
                style = MaterialTheme.typography.bodyLarge,
                modifier = Modifier.weight(1f),
            )
            Icon(
                imageVector = Icons.AutoMirrored.Filled.KeyboardArrowRight,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

// ── UI Components ────────────────────────────────────────────────────────

// One severity block (issues / warnings / info): a counted section header over a
// list of StatusBanners. No-op when [messages] is empty, so the three call sites
// can't drift in spacing or action wiring. [titleRes] takes the count as its arg.
@Composable
internal fun MessageSection(
    messages: List<DashboardMessage>,
    titleRes: Int,
    headerColor: Color,
    containerColor: Color,
    contentColor: Color,
    onOpenDiagnostics: () -> Unit,
    onContact: () -> Unit,
    onOpenHelp: (String?) -> Unit,
) {
    if (messages.isEmpty()) return
    Spacer(Modifier.height(20.dp))
    SectionHeader(stringResource(titleRes, messages.size), color = headerColor)
    Spacer(Modifier.height(8.dp))
    for (message in messages) {
        StatusBanner(
            text = message.text,
            containerColor = containerColor,
            contentColor = contentColor,
            action = messageActionSlot(message, onOpenDiagnostics, onContact, onOpenHelp),
        )
        Spacer(Modifier.height(6.dp))
    }
}

// Maps a message's data-layer action tag to the actual button + handler in ONE
// place, so a new action is an enum case + one branch here — not an edit in
// every message loop. The data layer stays UI-free (it only emits the tag).
// Precedence: a concrete fix (download the zip) first, then a tagged action,
// then the contextual "Learn more" link into the guide.
private fun messageActionSlot(
    message: DashboardMessage,
    onOpenDiagnostics: () -> Unit,
    onContactAuthor: () -> Unit,
    onOpenHelp: (String?) -> Unit,
): (@Composable () -> Unit)? {
    message.downloadArtifact?.let { artifact ->
        return { ModuleDownloadButton(artifact) }
    }
    when (message.action) {
        DashboardMessageAction.ContactAuthor -> return ({ ContactAuthorButton(onClick = onContactAuthor) })
        DashboardMessageAction.OpenDiagnostics -> return ({ DetailsButton(onClick = onOpenDiagnostics) })
        null -> Unit
    }
    message.helpArticle?.let { article ->
        return { LearnMoreButton(onClick = { onOpenHelp(article) }) }
    }
    return null
}

@Composable
private fun DetailsButton(onClick: () -> Unit) {
    EnhancedOutlinedButton(onClick = onClick) {
        Text(stringResource(R.string.dashboard_action_details))
    }
}

@Composable
private fun LearnMoreButton(onClick: () -> Unit) {
    EnhancedOutlinedButton(onClick = onClick) {
        Text(stringResource(R.string.dashboard_action_learn_more))
    }
}

@Composable
internal fun DashboardLoadingState(modifier: Modifier = Modifier) {
    val animations = LocalSettingsState.current.animationsEnabled
    val alpha =
        if (animations) {
            val transition = rememberInfiniteTransition(label = "dashboardLoading")
            val pulseAlpha by
                transition.animateFloat(
                    initialValue = 0.42f,
                    targetValue = 0.88f,
                    animationSpec =
                        infiniteRepeatable(
                            animation = tween(durationMillis = 900),
                            repeatMode = RepeatMode.Reverse,
                        ),
                    label = "dashboardLoadingAlpha",
                )
            pulseAlpha
        } else {
            0.72f
        }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
    ) {
        Spacer(Modifier.height(12.dp))
        EnhancedCard(
            modifier = Modifier.fillMaxWidth(),
            shape = MaterialTheme.shapes.extraLarge,
            color = AppColors.cardContainer,
        ) {
            Column(modifier = Modifier.padding(18.dp).fillMaxWidth()) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    LoadingBlock(
                        modifier = Modifier.size(58.dp).clip(CircleShape),
                        alpha = alpha,
                    )
                    Spacer(Modifier.width(16.dp))
                    Column(Modifier.weight(1f)) {
                        LoadingBlock(
                            modifier = Modifier.fillMaxWidth(0.56f).height(24.dp),
                            alpha = alpha,
                        )
                        Spacer(Modifier.height(8.dp))
                        LoadingBlock(
                            modifier = Modifier.fillMaxWidth(0.82f).height(14.dp),
                            alpha = alpha,
                        )
                    }
                }
                Spacer(Modifier.height(18.dp))
                LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                Spacer(Modifier.height(18.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    LoadingMetric(alpha = alpha, modifier = Modifier.weight(1f))
                    LoadingMetric(alpha = alpha, modifier = Modifier.weight(1f))
                }
                Spacer(Modifier.height(8.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    LoadingMetric(alpha = alpha, modifier = Modifier.weight(1f))
                    LoadingMetric(alpha = alpha, modifier = Modifier.weight(1f))
                }
            }
        }

        Spacer(Modifier.height(20.dp))
        LoadingSection(alpha = alpha, rows = 3)
        Spacer(Modifier.height(20.dp))
        LoadingSection(alpha = alpha, rows = 2)
        Spacer(Modifier.height(16.dp))
    }
}

@Composable
private fun LoadingMetric(
    alpha: Float,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier =
            modifier
                .clip(MaterialTheme.shapes.medium)
                .background(AppColors.cardContainerStrong)
                .padding(horizontal = 12.dp, vertical = 10.dp),
    ) {
        LoadingBlock(
            modifier = Modifier.fillMaxWidth(0.68f).height(12.dp),
            alpha = alpha,
        )
        Spacer(Modifier.height(8.dp))
        LoadingBlock(
            modifier = Modifier.fillMaxWidth(0.42f).height(18.dp),
            alpha = alpha,
        )
    }
}

@Composable
private fun LoadingSection(
    alpha: Float,
    rows: Int,
) {
    LoadingBlock(
        modifier = Modifier.fillMaxWidth(0.42f).height(18.dp),
        alpha = alpha,
    )
    Spacer(Modifier.height(8.dp))
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        repeat(rows) { index ->
            LoadingRow(index = index, count = rows, alpha = alpha)
        }
    }
}

@Composable
private fun LoadingRow(
    index: Int,
    count: Int,
    alpha: Float,
) {
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clip(MaterialTheme.shapes.medium)
                .background(AppColors.cardContainer)
                .padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        LoadingBlock(
            modifier = Modifier.size(40.dp),
            alpha = alpha,
        )
        Spacer(Modifier.width(16.dp))
        Column(Modifier.weight(1f)) {
            LoadingBlock(
                modifier = Modifier.fillMaxWidth(if (index == count - 1) 0.48f else 0.62f).height(16.dp),
                alpha = alpha,
            )
            Spacer(Modifier.height(8.dp))
            LoadingBlock(
                modifier = Modifier.fillMaxWidth(if (index == 0) 0.78f else 0.68f).height(12.dp),
                alpha = alpha,
            )
        }
    }
}

@Composable
private fun LoadingBlock(
    alpha: Float,
    modifier: Modifier = Modifier,
) {
    Box(
        modifier =
            modifier
                .clip(MaterialTheme.shapes.medium)
                .alpha(alpha)
                .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.18f)),
    )
}

/**
 * Dashboard section title: the emphasized (titleMedium) variant, defaulting to
 * onSurface. Pass [color] for the colored issue/warning headers. Delegates to
 * the shared [SharedSectionHeader] so the rendering isn't duplicated per screen.
 */
@Composable
private fun SectionHeader(
    text: String,
    color: Color = MaterialTheme.colorScheme.onSurface,
) {
    SharedSectionHeader(text = text, color = color, emphasized = true)
}

@Composable
private fun JavaBackendCard(
    state: LsposedState,
    index: Int = -1,
    count: Int = 1,
) {
    val name = stringResource(R.string.dashboard_java_backend)
    val lsposed = stringResource(R.string.dashboard_backend_lsposed)
    val installedVersion = BuildConfig.VERSION_NAME
    val (status, accentColor, accentContainerColor) =
        when (state) {
            is LsposedState.NotInstalled -> {
                Triple(
                    stringResource(R.string.dashboard_not_installed),
                    MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.55f),
                    AppColors.neutralAccentContainer,
                )
            }

            is LsposedState.InstalledInactive -> {
                Triple(
                    stringResource(R.string.dashboard_installed_inactive),
                    StatusColors.warningAccent,
                    StatusColors.warningContainer(),
                )
            }

            is LsposedState.NeedsReboot -> {
                Triple(
                    stringResource(R.string.dashboard_reboot_needed),
                    StatusColors.warningAccent,
                    StatusColors.warningContainer(),
                )
            }

            is LsposedState.Active -> {
                Triple(
                    stringResource(R.string.dashboard_active_targets, state.targetCount),
                    StatusColors.successDot,
                    StatusColors.successContainer(),
                )
            }
        }
    val base = stringResource(R.string.dashboard_backend_line, lsposed, status)
    val subtitle =
        if (state is LsposedState.Active && state.version != null) {
            base + "\n" + stringResource(R.string.dashboard_running_version, state.version)
        } else {
            base
        }
    ModuleCardShell(
        name = name,
        badgeText = "J",
        index = index,
        count = count,
        version = installedVersion,
        subtitle = subtitle,
        accentColor = accentColor,
        accentContainerColor = accentContainerColor,
    )
}

@Composable
private fun CoreComponentCard(
    core: CoreDashboardData,
    index: Int = 2,
    count: Int = 3,
) {
    val ready = core.health == CoreHealth.Ready
    val failed =
        core.health in
            setOf(
                CoreHealth.Unavailable,
                CoreHealth.ProtocolMismatch,
                CoreHealth.PermissionDenied,
                CoreHealth.StartupFault,
                CoreHealth.StorageFault,
            )
    val subtitle =
        buildString {
            append(stringResource(coreHealthResource(core.health)))
            core.detail?.let { append("\n").append(it) }
            append("\n").append(stringResource(R.string.dashboard_core_installed, core.installedVersion ?: "—"))
            append("\n").append(
                stringResource(
                    R.string.dashboard_core_ports,
                    stringResource(R.string.dashboard_ports),
                    core.portsTargets?.toString() ?: "—",
                    stringResource(corePortsResource(core.ports)),
                ),
            )
        }
    ModuleCardShell(
        name = "VPN Hide Core",
        badgeText = "C",
        version = null,
        subtitle = subtitle,
        accentColor =
            when {
                ready -> StatusColors.successDot
                failed -> StatusColors.errorDot
                else -> StatusColors.warningAccent
            },
        accentContainerColor =
            when {
                ready -> StatusColors.successContainer()
                failed -> StatusColors.errorContainer()
                else -> StatusColors.warningContainer()
            },
        index = index,
        count = count,
    )
}

@Composable
private fun ModuleCardShell(
    name: String,
    badgeText: String,
    version: String?,
    subtitle: String,
    accentColor: Color,
    accentContainerColor: Color,
    index: Int,
    count: Int,
) {
    GroupedCard(
        index = index,
        count = count,
        modifier = Modifier.fillMaxWidth(),
        color = AppColors.cardContainer,
    ) {
        Row(
            modifier = Modifier.padding(16.dp).fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ModuleBadge(text = badgeText, accentColor = accentColor, containerColor = accentContainerColor)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                ModuleHeading(name, version)
                Text(
                    text = subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Spacer(Modifier.width(10.dp))
            Box(
                modifier =
                    Modifier
                        .size(9.dp)
                        .clip(CircleShape)
                        .background(accentColor),
            )
        }
    }
}

@Composable
private fun ModuleBadge(
    text: String,
    accentColor: Color,
    containerColor: Color,
) {
    Box(
        modifier =
            Modifier
                .size(42.dp)
                .clip(MaterialTheme.shapes.medium)
                .background(containerColor),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = text,
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Bold,
            color = accentColor,
        )
    }
}

private const val GITHUB_RELEASES_PAGE = "https://github.com/okhsunrog/vpnhide/releases"

/** Direct download of an asset from the latest release. GitHub redirects
 * `/releases/latest/download/<asset>` to the current release's asset, so the URL
 * stays correct without hard-coding a tag or version — we always point at the
 * latest published release (§ releasing.md). */
private fun releaseAssetUrl(artifact: String): String = "$GITHUB_RELEASES_PAGE/latest/download/$artifact"

/** Compact "download this zip" button for a module-problem banner (wrong variant,
 * outdated version, …) — grabs the exact named artifact from the latest release. */
@Composable
internal fun ModuleDownloadButton(artifact: String) {
    val uriHandler = LocalUriHandler.current
    EnhancedOutlinedButton(onClick = { runCatching { uriHandler.openUri(releaseAssetUrl(artifact)) } }) {
        Text(stringResource(R.string.dashboard_install_recommendation_download, artifact))
    }
}

@Composable
private fun DashboardLoadErrorCard(
    title: String,
    message: String,
    containerColor: Color,
    titleColor: Color,
    contentColor: Color,
    onRetry: () -> Unit,
) {
    EnhancedCard(
        modifier = Modifier.fillMaxWidth(),
        color = containerColor,
        shape = MaterialTheme.shapes.medium,
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(
                text = title,
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
                color = titleColor,
            )
            Spacer(Modifier.height(6.dp))
            Text(
                text = message,
                style = MaterialTheme.typography.bodyMedium,
                color = contentColor,
            )
            Spacer(Modifier.height(12.dp))
            EnhancedButton(onClick = onRetry) {
                Text(stringResource(R.string.retry))
            }
        }
    }
}

// ── Update & Changelog ──────────────────────────────────────────────────

@Composable
private fun UpdateAvailableCard(info: UpdateInfo) {
    val context = LocalContext.current
    EnhancedCard(
        modifier = Modifier.fillMaxWidth(),
        color = StatusColors.infoContainer(),
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
            EnhancedButton(
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
    // Non-empty by construction — the caller only shows this dialog when
    // changelog history has entries (see the load effect above).
    val entries = remember(data) { data.history }
    var index by remember { mutableIntStateOf(0) }
    val entry = entries[index]
    val locale =
        LocalConfiguration.current.locales[0]
            .language
    val sectionLabels =
        mapOf(
            "added" to stringResource(R.string.changelog_section_added),
            "changed" to stringResource(R.string.changelog_section_changed),
            "fixed" to stringResource(R.string.changelog_section_fixed),
            "removed" to stringResource(R.string.changelog_section_removed),
            "deprecated" to stringResource(R.string.changelog_section_deprecated),
            "security" to stringResource(R.string.changelog_section_security),
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
