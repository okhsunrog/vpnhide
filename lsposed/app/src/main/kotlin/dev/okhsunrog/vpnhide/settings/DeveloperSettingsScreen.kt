package dev.okhsunrog.vpnhide.settings

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.BugReport
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Update
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.CanonicalPreferenceSwitch
import dev.okhsunrog.vpnhide.CanonicalToggle
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.ui.components.PreferenceRowSwitch
import dev.okhsunrog.vpnhide.ui.theme.AppColors

/**
 * Settings → For developers: the switches that only make sense to someone
 * building the project rather than using it.
 *
 * Its own page rather than a section at the bottom of Settings, because the list
 * is expected to keep growing and none of it is relevant to a normal install.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun DeveloperSettingsScreen(onBack: () -> Unit) {
    val settings = LocalSettingsState.current
    val interactor = LocalSettingsInteractor.current
    BackHandler(onBack = onBack)
    Scaffold(
        containerColor = AppColors.screenBackground,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.settings_developer_section)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = stringResource(R.string.action_back),
                        )
                    }
                },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = AppColors.topBarContainer,
                        titleContentColor = MaterialTheme.colorScheme.onSurface,
                        navigationIconContentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                    ),
            )
        },
    ) { padding ->
        Column(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .verticalScroll(rememberScrollState())
                    .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            PreferenceRowSwitch(
                title = stringResource(R.string.settings_suppress_changelog),
                subtitle = stringResource(R.string.settings_suppress_changelog_sub),
                icon = Icons.Default.History,
                index = 0,
                count = ROW_COUNT,
                checked = settings.suppressChangelog,
                onCheckedChange = interactor::setSuppressChangelog,
            )
            PreferenceRowSwitch(
                title = stringResource(R.string.settings_ignore_version_mismatch),
                subtitle = stringResource(R.string.settings_ignore_version_mismatch_sub),
                icon = Icons.Default.Update,
                index = 1,
                count = ROW_COUNT,
                checked = settings.ignoreVersionMismatch,
                onCheckedChange = interactor::setIgnoreVersionMismatch,
            )
            // Off by default. The bridge ships in release too (the user develops on
            // release builds) — when on it opens a loopback control port, which the
            // dashboard surfaces as an info note so it isn't left running unnoticed.
            PreferenceRowSwitch(
                title = stringResource(R.string.settings_agent_control),
                subtitle = stringResource(R.string.settings_agent_control_sub),
                icon = Icons.Default.Settings,
                index = 2,
                count = ROW_COUNT,
                checked = settings.agentControlEnabled,
                onCheckedChange = interactor::setAgentControlEnabled,
            )
            CanonicalPreferenceSwitch(
                field = CanonicalToggle.DebugSwitch,
                title = stringResource(R.string.settings_debug_logging),
                subtitle = stringResource(R.string.settings_debug_logging_sub),
                icon = Icons.Default.BugReport,
                index = 3,
                count = ROW_COUNT,
            )
        }
    }
}

private const val ROW_COUNT = 4
