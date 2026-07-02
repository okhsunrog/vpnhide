package dev.okhsunrog.vpnhide

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
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.settings.LocalSettingsState
import dev.okhsunrog.vpnhide.settings.SettingsRepository
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard
import dev.okhsunrog.vpnhide.ui.components.EnhancedOutlinedButton
import dev.okhsunrog.vpnhide.ui.components.GroupedCard
import dev.okhsunrog.vpnhide.ui.components.IconBubble
import dev.okhsunrog.vpnhide.ui.components.MetricTile
import dev.okhsunrog.vpnhide.ui.components.pulse
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
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    val state by DashboardCache.state.collectAsState()
    val loadError by DashboardCache.error.collectAsState()
    val updateInfo by UpdateCheckCache.info.collectAsState()
    var showChangelog by remember { mutableStateOf(false) }
    var changelogData by remember { mutableStateOf<ChangelogData?>(null) }
    var showContact by remember { mutableStateOf(false) }

    if (showContact) {
        ContactModal(onDismiss = { showContact = false })
    }

    // Both caches are reactive to tab switches without re-doing work:
    // ensureLoaded / ensureFresh are no-ops if the data is already
    // populated or an inflight job hasn't finished yet.
    LaunchedEffect(state == null && loadError == null, selfNeedsRestart) {
        if (state == null && loadError == null) {
            DashboardCache.ensureLoaded(scope, context, selfNeedsRestart)
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
            SettingsRepository(context.applicationContext).settings.first().suppressVersionWarnings
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

    if (state == null && loadError == null) {
        DashboardLoadingState(modifier = modifier)
        return
    }

    Column(
        modifier =
            modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
    ) {
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
                onRetry = { DashboardCache.refresh(scope, context, selfNeedsRestart) },
            )
            Spacer(Modifier.height(12.dp))
            if (s == null) return@Column
        }
        val loadedState = s ?: return@Column

        // Messages split by severity. Only errors/warnings affect the hero:
        // info messages are neutral notes rendered below without changing the
        // overall "Protected" state or issue count.
        val errors = loadedState.messages.filter { it.severity == DashboardMessageSeverity.ERROR }
        val warnings = loadedState.messages.filter { it.severity == DashboardMessageSeverity.WARNING }
        val infos = loadedState.messages.filter { it.severity == DashboardMessageSeverity.INFO }

        // Hero: the whole setup's health at a glance.
        DashboardHeroCard(state = loadedState, errorCount = errors.size, warningCount = warnings.size)

        // Critical protection states sit right under the hero (not in a separate
        // mid-screen section): the VPN needs turning on, or a self-restart is
        // pending. The all-good "Checked" state renders nothing here — the hero's
        // per-level tiles already carry that status, so the old duplicate per-level
        // cards (Native / Java «OK», which just restated those tiles) are gone.
        when (loadedState.protection) {
            is ProtectionCheck.NoVpn -> {
                Spacer(Modifier.height(12.dp))
                VpnOffPrompt(
                    onRetry = {
                        // Re-read dashboard state (re-runs its own VPN + protection
                        // probes) and re-run the diag cache so both screens move to
                        // "Ready" when VPN is back.
                        DashboardCache.refresh(scope, context, selfNeedsRestart)
                        DiagnosticsCache.retry(scope, context)
                    },
                )
            }

            is ProtectionCheck.NeedsRestart -> {
                Spacer(Modifier.height(12.dp))
                StatusBanner(
                    text = stringResource(R.string.dashboard_needs_restart),
                    containerColor = warningBg,
                    contentColor = onBannerColor,
                )
            }

            is ProtectionCheck.Checked -> {
                Unit
            }
        }
        Spacer(Modifier.height(20.dp))

        // Module status cards — one grouped block (byIndex corners).
        SectionHeader(stringResource(R.string.dashboard_modules))
        Spacer(Modifier.height(8.dp))
        // Two layers + ports: the always-on Java backend (LSPosed) and the one
        // active native backend (kmod / KPM / Zygisk, §1.5), then the separate
        // ports feature.
        Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
            JavaBackendCard(loadedState.lsposed, index = 0, count = 3)
            NativeBackendCard(loadedState.nativeBackend, selfNeedsRestart, index = 1, count = 3)
            ModuleCard(stringResource(R.string.dashboard_ports), "P", loadedState.ports, index = 2, count = 3)
        }
        loadedState.nativeInstallRecommendation?.let { recommendation ->
            Spacer(Modifier.height(8.dp))
            NativeInstallRecommendationCard(recommendation)
        }
        updateInfo?.let { info ->
            Spacer(Modifier.height(8.dp))
            UpdateAvailableCard(info)
        }

        if (errors.isNotEmpty()) {
            Spacer(Modifier.height(20.dp))
            SectionHeader(stringResource(R.string.dashboard_issues, errors.size), color = errorHeader)
            Spacer(Modifier.height(8.dp))
            for (issue in errors) {
                StatusBanner(
                    text = issue.text,
                    containerColor = errorBg,
                    contentColor = onBannerColor,
                    action = messageActionSlot(issue.action, onOpenDiagnostics) { showContact = true },
                )
                Spacer(Modifier.height(6.dp))
            }
        }

        if (warnings.isNotEmpty()) {
            Spacer(Modifier.height(20.dp))
            SectionHeader(stringResource(R.string.dashboard_warnings, warnings.size), color = warningHeader)
            Spacer(Modifier.height(8.dp))
            for (issue in warnings) {
                StatusBanner(
                    text = issue.text,
                    containerColor = warningBg,
                    contentColor = onBannerColor,
                    action = messageActionSlot(issue.action, onOpenDiagnostics) { showContact = true },
                )
                Spacer(Modifier.height(6.dp))
            }
        }

        if (infos.isNotEmpty()) {
            Spacer(Modifier.height(20.dp))
            SectionHeader(stringResource(R.string.dashboard_info, infos.size), color = infoHeader)
            Spacer(Modifier.height(8.dp))
            for (message in infos) {
                StatusBanner(
                    text = message.text,
                    containerColor = infoBg,
                    contentColor = onBannerColor,
                    action = messageActionSlot(message.action, onOpenDiagnostics) { showContact = true },
                )
                Spacer(Modifier.height(6.dp))
            }
        }

        Spacer(Modifier.height(16.dp))
    }
}

// ── UI Components ────────────────────────────────────────────────────────

// Maps a message's data-layer action tag to the actual button + handler in ONE
// place, so a new action is an enum case + one branch here — not an edit in
// every message loop. The data layer stays UI-free (it only emits the tag).
private fun messageActionSlot(
    action: DashboardMessageAction?,
    onOpenDiagnostics: () -> Unit,
    onContactAuthor: () -> Unit,
): (@Composable () -> Unit)? =
    when (action) {
        DashboardMessageAction.ContactAuthor -> ({ ContactAuthorButton(onClick = onContactAuthor) })
        DashboardMessageAction.OpenDiagnostics -> ({ DetailsButton(onClick = onOpenDiagnostics) })
        null -> null
    }

@Composable
private fun DetailsButton(onClick: () -> Unit) {
    EnhancedOutlinedButton(onClick = onClick) {
        Text(stringResource(R.string.dashboard_action_details))
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

private data class HeroVisual(
    val container: Color,
    val accent: Color,
    val icon: ImageVector,
    val titleRes: Int,
    val subtitleRes: Int,
)

/**
 * The big at-a-glance status card at the top of the Dashboard. Summarizes the
 * whole setup's health into one of four states with a tinted container, accent
 * icon and headline; the icon breathes when fully protected.
 */
@Composable
private fun DashboardHeroCard(
    state: DashboardState,
    errorCount: Int,
    warningCount: Int,
) {
    val animations = LocalSettingsState.current.animationsEnabled
    val status = computeHeroStatus(state, errorCount, warningCount)
    val visual =
        when (status) {
            HeroStatus.Protected -> {
                HeroVisual(
                    container = StatusColors.successContainer(),
                    accent = StatusColors.successDot,
                    icon = Icons.Default.Shield,
                    titleRes = R.string.dashboard_hero_protected_title,
                    subtitleRes = R.string.dashboard_hero_protected_subtitle,
                )
            }

            HeroStatus.Attention -> {
                HeroVisual(
                    container = StatusColors.warningContainer(),
                    accent = StatusColors.warningAccent,
                    icon = Icons.Default.Warning,
                    titleRes = R.string.dashboard_hero_attention_title,
                    subtitleRes = R.string.dashboard_hero_attention_subtitle,
                )
            }

            HeroStatus.Unprotected -> {
                HeroVisual(
                    container = StatusColors.errorContainer(),
                    accent = StatusColors.errorAccent,
                    icon = Icons.Default.Warning,
                    titleRes = R.string.dashboard_hero_unprotected_title,
                    subtitleRes = R.string.dashboard_hero_unprotected_subtitle,
                )
            }

            HeroStatus.VpnOff -> {
                HeroVisual(
                    container = MaterialTheme.colorScheme.surfaceVariant,
                    accent = MaterialTheme.colorScheme.onSurfaceVariant,
                    icon = Icons.Default.Info,
                    titleRes = R.string.dashboard_hero_vpnoff_title,
                    subtitleRes = R.string.dashboard_hero_vpnoff_subtitle,
                )
            }
        }
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
                IconBubble(
                    icon = visual.icon,
                    tint = visual.accent,
                    container = visual.container,
                    modifier =
                        Modifier.pulse(
                            enabled = status == HeroStatus.Protected && animations,
                            min = 0.94f,
                            max = 1.05f,
                            durationMillis = 1300,
                        ),
                )
                Spacer(Modifier.width(16.dp))
                Column(Modifier.weight(1f)) {
                    Text(
                        text = stringResource(visual.titleRes),
                        style = MaterialTheme.typography.headlineSmall,
                        fontWeight = FontWeight.Bold,
                    )
                    Spacer(Modifier.height(2.dp))
                    Text(
                        text = stringResource(visual.subtitleRes),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Spacer(Modifier.height(16.dp))
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    MetricTile(
                        label = stringResource(R.string.dashboard_summary_modules),
                        value = moduleSummaryText(state),
                        accent = moduleSummaryAccent(state),
                        modifier = Modifier.weight(1f),
                    )
                    MetricTile(
                        label = stringResource(R.string.dashboard_native_protection),
                        value = nativeSummaryText(state.protection),
                        accent = nativeSummaryAccent(state.protection),
                        modifier = Modifier.weight(1f),
                    )
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    MetricTile(
                        label = stringResource(R.string.dashboard_java_protection),
                        value = javaSummaryText(state.protection),
                        accent = javaSummaryAccent(state.protection),
                        modifier = Modifier.weight(1f),
                    )
                    MetricTile(
                        label = stringResource(R.string.dashboard_summary_issues),
                        value = (errorCount + warningCount).toString(),
                        accent =
                            when {
                                errorCount > 0 -> StatusColors.errorAccent
                                warningCount > 0 -> StatusColors.warningAccent
                                else -> StatusColors.successDot
                            },
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        }
    }
}

@Composable
private fun moduleSummaryAccent(state: DashboardState): Color {
    val nativeActive = moduleActive(state.nativeBackend.state)
    val nativeBroken = (state.nativeBackend.state as? ModuleState.Installed)?.brokenReason != null
    return when {
        nativeBroken -> StatusColors.errorAccent
        state.lsposed is LsposedState.Active && nativeActive -> StatusColors.successDot
        activeModuleCount(state) > 0 -> StatusColors.warningAccent
        else -> StatusColors.errorAccent
    }
}

@Composable
private fun nativeSummaryText(protection: ProtectionCheck): String =
    when (protection) {
        ProtectionCheck.NoVpn -> {
            stringResource(R.string.dashboard_hero_vpnoff_title)
        }

        ProtectionCheck.NeedsRestart -> {
            stringResource(R.string.dashboard_protection_unknown)
        }

        is ProtectionCheck.Checked -> {
            when (val native = protection.native) {
                NativeResult.Ok -> {
                    stringResource(R.string.dashboard_protection_ok)
                }

                is NativeResult.Fail -> {
                    if (native.passed > 0) {
                        stringResource(R.string.dashboard_protection_partial)
                    } else {
                        stringResource(R.string.dashboard_protection_fail)
                    }
                }

                NativeResult.NoModule -> {
                    stringResource(R.string.dashboard_protection_no_module)
                }
            }
        }
    }

@Composable
private fun nativeSummaryAccent(protection: ProtectionCheck): Color =
    when (protection) {
        ProtectionCheck.NoVpn -> {
            StatusColors.infoAccent
        }

        ProtectionCheck.NeedsRestart -> {
            StatusColors.warningAccent
        }

        is ProtectionCheck.Checked -> {
            when (val native = protection.native) {
                NativeResult.Ok -> StatusColors.successDot
                is NativeResult.Fail -> if (native.passed > 0) StatusColors.warningAccent else StatusColors.errorAccent
                NativeResult.NoModule -> MaterialTheme.colorScheme.onSurfaceVariant
            }
        }
    }

@Composable
private fun javaSummaryText(protection: ProtectionCheck): String =
    when (protection) {
        ProtectionCheck.NoVpn -> {
            stringResource(R.string.dashboard_hero_vpnoff_title)
        }

        ProtectionCheck.NeedsRestart -> {
            stringResource(R.string.dashboard_protection_unknown)
        }

        is ProtectionCheck.Checked -> {
            when (protection.java) {
                JavaResult.Ok -> stringResource(R.string.dashboard_protection_ok)
                is JavaResult.Fail -> stringResource(R.string.dashboard_protection_fail)
                JavaResult.HooksInactive -> stringResource(R.string.dashboard_protection_hooks_inactive)
            }
        }
    }

@Composable
private fun javaSummaryAccent(protection: ProtectionCheck): Color =
    when (protection) {
        ProtectionCheck.NoVpn -> {
            StatusColors.infoAccent
        }

        ProtectionCheck.NeedsRestart -> {
            StatusColors.warningAccent
        }

        is ProtectionCheck.Checked -> {
            when (protection.java) {
                JavaResult.Ok -> StatusColors.successDot
                is JavaResult.Fail -> StatusColors.errorAccent
                JavaResult.HooksInactive -> MaterialTheme.colorScheme.onSurfaceVariant
            }
        }
    }

private data class InstalledVisual(
    val subtitle: String,
    val accentColor: Color,
    val accentContainerColor: Color,
)

/**
 * Status subtitle + colors for an installed flashable module. Shared by the
 * Ports card and the Native-backend card so the broken / active / inactive
 * logic lives in one place (keeps the cards in sync and CPD quiet).
 */
@Composable
private fun installedVisual(
    state: ModuleState.Installed,
    selfNeedsRestart: Boolean,
): InstalledVisual {
    val active = state.active
    val broken = state.brokenReason
    val brokenSubtitleRes =
        when (broken) {
            KmodBrokenReason.WrongVariant -> R.string.dashboard_kmod_broken_wrong_variant
            KmodBrokenReason.UnsupportedKernel -> R.string.dashboard_kmod_broken_unsupported_kernel
            KmodBrokenReason.MissingKprobes -> R.string.dashboard_kmod_broken_no_kprobes
            KmodBrokenReason.UnknownVariantInactive -> R.string.dashboard_kmod_broken_unknown_variant
            KmodBrokenReason.AmbiguousLoadFailed -> R.string.dashboard_kmod_broken_ambiguous
            KmodBrokenReason.SignatureEnforced -> R.string.dashboard_kmod_broken_signature_enforced
            null -> null
        }
    return InstalledVisual(
        subtitle =
            when {
                brokenSubtitleRes != null -> stringResource(brokenSubtitleRes)
                active -> stringResource(R.string.dashboard_active_targets, state.targetCount)
                selfNeedsRestart -> stringResource(R.string.dashboard_installed_restart_app)
                else -> stringResource(R.string.dashboard_installed_inactive)
            },
        accentColor =
            when {
                broken != null -> StatusColors.errorDot
                active -> StatusColors.successDot
                else -> StatusColors.warningAccent
            },
        accentContainerColor =
            when {
                broken != null -> StatusColors.errorContainer()
                active -> StatusColors.successContainer()
                else -> StatusColors.warningContainer()
            },
    )
}

@Composable
private fun ModuleCard(
    name: String,
    badgeText: String,
    state: ModuleState,
    selfNeedsRestart: Boolean = false,
    index: Int = -1,
    count: Int = 1,
) {
    when (state) {
        is ModuleState.NotInstalled -> {
            ModuleCardShell(
                name = name,
                badgeText = badgeText,
                index = index,
                count = count,
                version = null,
                subtitle = stringResource(R.string.dashboard_not_installed),
                accentColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.55f),
                accentContainerColor = AppColors.neutralAccentContainer,
            )
        }

        is ModuleState.Installed -> {
            val v = installedVisual(state, selfNeedsRestart)
            ModuleCardShell(
                name = name,
                badgeText = badgeText,
                index = index,
                count = count,
                version = state.version,
                subtitle = v.subtitle,
                accentColor = v.accentColor,
                accentContainerColor = v.accentContainerColor,
            )
        }
    }
}

/** The always-on Java backend (LSPosed). Badge "J"; subtitle is "LSPosed · …". */
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

/**
 * The native backend surfaced on Dashboard. It is the active backend when one
 * exists, otherwise the highest-priority installed backend so inactive installs
 * stay visible. The restart-app hint only applies to Zygisk.
 */
@Composable
private fun NativeBackendCard(
    backend: DisplayNativeBackend,
    selfNeedsRestart: Boolean,
    index: Int = -1,
    count: Int = 1,
) {
    val name = stringResource(R.string.dashboard_native_backend)
    val state = backend.state
    if (backend.id == null || state !is ModuleState.Installed) {
        ModuleCardShell(
            name = name,
            badgeText = "N",
            index = index,
            count = count,
            version = null,
            subtitle = stringResource(R.string.dashboard_not_installed),
            accentColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.55f),
            accentContainerColor = AppColors.neutralAccentContainer,
        )
        return
    }
    val backendName =
        stringResource(
            when (backend.id) {
                NativeBackendId.Kmod -> R.string.dashboard_backend_kmod
                NativeBackendId.Kpm -> R.string.dashboard_backend_kpm
                NativeBackendId.Zygisk -> R.string.dashboard_backend_zygisk
            },
        )
    val v = installedVisual(state, selfNeedsRestart && backend.id == NativeBackendId.Zygisk)
    ModuleCardShell(
        name = name,
        badgeText = "N",
        index = index,
        count = count,
        version = state.version,
        subtitle = stringResource(R.string.dashboard_backend_line, backendName, v.subtitle),
        accentColor = v.accentColor,
        accentContainerColor = v.accentContainerColor,
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
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                    Text(
                        text = name,
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.Bold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    if (version != null) {
                        Spacer(Modifier.width(8.dp))
                        Text(
                            text = version,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.widthIn(max = 118.dp),
                        )
                    }
                }
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

@Composable
private fun NativeInstallRecommendationCard(recommendation: NativeInstallRecommendation) {
    val containerColor =
        if (recommendation.recommended == RecommendedBackend.Zygisk) {
            StatusColors.zygiskRecommendContainer()
        } else {
            StatusColors.infoContainer()
        }

    EnhancedCard(
        modifier = Modifier.fillMaxWidth(),
        color = containerColor,
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
            // Disambiguate the GKI KMI tag baked into uname -r (e.g.
            // "android12-5.10") from the device's Android OS release on
            // devices where they differ — common on old Pixels still on
            // an android12 KMI kernel under an Android 14/15 ROM. Hide
            // the note when both match (would just be noise) or when
            // uname -r carries no KMI tag at all.
            val kmiBranch = recommendation.kernelBranch
            if (kmiBranch != null && kmiBranch != recommendation.androidVersion) {
                Spacer(Modifier.height(4.dp))
                Text(
                    text =
                        stringResource(
                            R.string.dashboard_install_recommendation_kmi_note,
                            kmiBranch.replace(" ", "").lowercase(),
                        ),
                    style = MaterialTheme.typography.bodySmall,
                    fontStyle = FontStyle.Italic,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Spacer(Modifier.height(8.dp))
            val alternative = recommendation.alternativeArtifact
            Text(
                text =
                    when (recommendation.recommended) {
                        RecommendedBackend.Zygisk -> {
                            stringResource(
                                R.string.dashboard_install_recommendation_zygisk,
                                recommendation.recommendedArtifact,
                            )
                        }

                        RecommendedBackend.Kpm -> {
                            if (recommendation.kpatchRuntimeAvailable) {
                                stringResource(
                                    R.string.dashboard_install_recommendation_kpm,
                                    recommendation.recommendedArtifact,
                                )
                            } else {
                                stringResource(
                                    R.string.dashboard_install_recommendation_kpm_needs_runtime,
                                    recommendation.recommendedArtifact,
                                )
                            }
                        }

                        RecommendedBackend.Kmod -> {
                            if (recommendation.variantAmbiguous && alternative != null) {
                                stringResource(
                                    R.string.dashboard_install_recommendation_kmod_ambiguous,
                                    recommendation.recommendedArtifact,
                                    alternative,
                                )
                            } else {
                                stringResource(
                                    R.string.dashboard_install_recommendation_kmod,
                                    recommendation.recommendedArtifact,
                                )
                            }
                        }
                    },
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
            )
            // Trailing note per backend: zygisk's detectability caveat, kmod's
            // "stable; KPM is a universal beta alternative" mention, or KPM's
            // beta disclaimer.
            val note =
                when (recommendation.recommended) {
                    RecommendedBackend.Zygisk -> R.string.dashboard_install_recommendation_zygisk_warning
                    RecommendedBackend.Kmod -> R.string.dashboard_install_recommendation_kmod_kpm_alt
                    RecommendedBackend.Kpm -> R.string.dashboard_install_recommendation_kpm_beta_note
                }
            Spacer(Modifier.height(8.dp))
            Text(
                text = stringResource(note),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.8f),
            )
        }
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
                Text(stringResource(R.string.vpn_off_retry))
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
