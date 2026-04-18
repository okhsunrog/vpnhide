package dev.okhsunrog.vpnhide

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp

internal enum class ProtectionMode { VpnTargets, AppHiding, PortHiding }

@Composable
fun ProtectionScreen(
    searchQuery: String,
    showSystem: Boolean,
    showRussianOnly: Boolean,
    modifier: Modifier = Modifier,
) {
    var mode by rememberSaveable { mutableStateOf(ProtectionMode.VpnTargets) }

    Column(modifier = modifier.fillMaxSize()) {
        ProtectionModeSwitcher(
            mode = mode,
            onModeChange = { mode = it },
        )
        when (mode) {
            ProtectionMode.VpnTargets -> {
                AppPickerScreen(
                    searchQuery = searchQuery,
                    showSystem = showSystem,
                    showRussianOnly = showRussianOnly,
                    modifier = Modifier.fillMaxSize(),
                )
            }

            ProtectionMode.AppHiding -> {
                AppHidingScreen(
                    searchQuery = searchQuery,
                    showSystem = showSystem,
                    showRussianOnly = showRussianOnly,
                    modifier = Modifier.fillMaxSize(),
                )
            }

            ProtectionMode.PortHiding -> {
                PortsHidingScreen(
                    searchQuery = searchQuery,
                    showSystem = showSystem,
                    showRussianOnly = showRussianOnly,
                    modifier = Modifier.fillMaxSize(),
                )
            }
        }
    }
}

@Composable
private fun ProtectionModeSwitcher(
    mode: ProtectionMode,
    onModeChange: (ProtectionMode) -> Unit,
) {
    val options =
        listOf(
            ProtectionMode.VpnTargets to R.string.mode_vpn_targets,
            ProtectionMode.AppHiding to R.string.mode_app_hiding,
            ProtectionMode.PortHiding to R.string.mode_port_hiding,
        )
    SingleChoiceSegmentedButtonRow(
        modifier =
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 8.dp),
    ) {
        options.forEachIndexed { index, (m, labelRes) ->
            SegmentedButton(
                selected = m == mode,
                onClick = { onModeChange(m) },
                shape = SegmentedButtonDefaults.itemShape(index = index, count = options.size),
                icon = {},
            ) {
                Text(stringResource(labelRes))
            }
        }
    }
}
