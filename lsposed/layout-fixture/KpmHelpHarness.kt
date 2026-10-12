package dev.okhsunrog.vpnhide.settings

import android.os.Bundle
import android.widget.Toast
import androidx.activity.compose.setContent
import androidx.appcompat.app.AppCompatActivity
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.*
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.diagnostics.KpmProblemKind
import dev.okhsunrog.vpnhide.help.HelpScreen
import dev.okhsunrog.vpnhide.ui.theme.VpnHideTheme

/** Opt-in preview: uses the real message renderer, action and help UI, without root writes. */
class KpmHelpHarness : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            CompositionLocalProvider(LocalSettingsState provides AppSettings()) {
                VpnHideTheme {
                    val context = LocalContext.current
                    var help by rememberSaveable { mutableStateOf<String?>(null) }
                    Surface(Modifier.fillMaxSize()) {
                        if (help != null) {
                            HelpScreen(
                                initialArticleId = help,
                                onClose = { help = null },
                                onNavigate = { Toast.makeText(context, "Preview: $it", Toast.LENGTH_SHORT).show() },
                            )
                        } else {
                            Column(
                                Modifier
                                    .fillMaxSize()
                                    .systemBarsPadding()
                                    .verticalScroll(rememberScrollState())
                                    .padding(16.dp),
                            ) {
                                Text("UI preview · KPM runtime missing", style = MaterialTheme.typography.titleLarge)
                                Text("Test data only · реальные модули и настройки не меняются")
                                val message =
                                    DashboardIssue
                                        .ModuleBroken(ModuleProblem.Kpm(KpmProblemKind.NoKernelPatchRuntime))
                                        .toMessage(context, context.resources)
                                MessageSection(
                                    messages = listOf(message),
                                    titleRes = R.string.dashboard_issues,
                                    headerColor = StatusColors.errorHeader(),
                                    containerColor = StatusColors.errorContainer(),
                                    contentColor = MaterialTheme.colorScheme.onSurface,
                                    onOpenDiagnostics = {},
                                    onContact = {},
                                    onOpenHelp = { help = it },
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}
