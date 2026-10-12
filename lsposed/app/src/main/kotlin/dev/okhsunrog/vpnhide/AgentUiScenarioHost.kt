package dev.okhsunrog.vpnhide

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.okhsunrog.vpnhide.help.HelpScreen

/** One host for every scenario. The real app remains composed underneath, including editor drafts. */
@Composable
internal fun AgentUiScenarioHost() {
    val state by agentUiSession.state.collectAsStateWithLifecycle()
    if (state.scenario == null) return
    val act: (() -> Unit) -> Unit = { operation ->
        agentUiSession.interact(state.sessionId, state.revision, operation)
    }
    Dialog(
        onDismissRequest = { act { agentUiSession.back(state.sessionId) } },
        properties = DialogProperties(usePlatformDefaultWidth = false, dismissOnClickOutside = false),
    ) {
        Surface(Modifier.fillMaxSize()) {
            Column {
                Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp), horizontalArrangement = Arrangement.SpaceBetween) {
                    Text(
                        stringResource(R.string.agent_ui_preview_label, state.scenario.orEmpty()),
                        modifier = Modifier.weight(1f).padding(vertical = 12.dp),
                        style = MaterialTheme.typography.labelMedium,
                    )
                    TextButton(onClick = { act { agentUiSession.stop(state.sessionId) } }) {
                        Text(stringResource(R.string.agent_ui_preview_exit))
                    }
                }
                HorizontalDivider()
                state.lastAction?.let {
                    Text(stringResource(R.string.agent_ui_preview_action, it), Modifier.padding(horizontal = 12.dp))
                }
                key(state.sessionId, state.revision) {
                    val uriHandler =
                        remember(state.sessionId, state.revision) {
                            object : UriHandler {
                                override fun openUri(uri: String) {
                                    agentUiSession.observe(state.sessionId, state.revision, action = uri)
                                }
                            }
                        }
                    CompositionLocalProvider(LocalUriHandler provides uriHandler) {
                        AgentUiScenarioContent(state, act)
                    }
                }
            }
        }
    }
}

@Composable
private fun AgentUiScenarioContent(
    state: AgentUiState,
    act: (() -> Unit) -> Unit,
) {
    SideEffect { agentUiSession.observe(state.sessionId, state.revision) }
    if (state.helpOpen) {
        HelpScreen(
            initialArticleId = state.destination,
            onClose = { act { agentUiSession.back(state.sessionId) } },
            onNavigate = { agentUiSession.observe(state.sessionId, state.revision, action = it) },
            onLocationChanged = { agentUiSession.observe(state.sessionId, state.revision, destination = it) },
        )
        return
    }
    if (state.scenario == "dashboard-loading") {
        DashboardLoadingState()
        return
    }
    val context = LocalContext.current
    val issue = agentUiIssues[state.scenario] ?: return
    val message = issue.toMessage(context, context.resources)
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
        val severity = message.severity
        MessageSection(
            messages = listOf(message),
            titleRes =
                when (severity) {
                    DashboardMessageSeverity.ERROR -> R.string.dashboard_issues
                    DashboardMessageSeverity.WARNING -> R.string.dashboard_warnings
                    DashboardMessageSeverity.INFO -> R.string.dashboard_info
                },
            headerColor =
                when (severity) {
                    DashboardMessageSeverity.ERROR -> StatusColors.errorHeader()
                    DashboardMessageSeverity.WARNING -> StatusColors.warningHeader()
                    DashboardMessageSeverity.INFO -> StatusColors.neutralHeader()
                },
            containerColor =
                when (severity) {
                    DashboardMessageSeverity.ERROR -> StatusColors.errorContainer()
                    DashboardMessageSeverity.WARNING -> StatusColors.warningContainer()
                    DashboardMessageSeverity.INFO -> StatusColors.neutralContainer()
                },
            contentColor = MaterialTheme.colorScheme.onSurface,
            onOpenDiagnostics = { agentUiSession.observe(state.sessionId, state.revision, action = "vpnhide://diagnostics") },
            onContact = { agentUiSession.observe(state.sessionId, state.revision, action = "contact") },
            onOpenHelp = { destination -> act { agentUiSession.navigate(AgentUiNavigationArgs(state.sessionId, destination)) } },
        )
    }
}
