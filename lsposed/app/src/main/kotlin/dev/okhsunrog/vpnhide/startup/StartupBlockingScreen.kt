package dev.okhsunrog.vpnhide.startup

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.ui.components.BlockingErrorCard
import dev.okhsunrog.vpnhide.ui.theme.AppColors

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun StartupBlockingScreen(
    title: String,
    body: String,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    Scaffold(
        containerColor = AppColors.screenBackground,
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.app_name)) },
                colors =
                    TopAppBarDefaults.topAppBarColors(
                        containerColor = AppColors.topBarContainer,
                        titleContentColor = MaterialTheme.colorScheme.onSurface,
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
            BlockingErrorCard(
                icon = Icons.Default.Lock,
                title = title,
                body = body,
                actionLabel = actionLabel,
                onAction = onAction,
            )
        }
    }
}
