package dev.okhsunrog.vpnhide

import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import dev.okhsunrog.vpnhide.ui.components.PreferenceRowSwitch
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch

@Composable
internal fun CanonicalPreferenceSwitch(
    field: CanonicalToggle,
    title: String,
    subtitle: String,
    icon: ImageVector,
    index: Int = -1,
    count: Int = 1,
    enabled: Boolean = true,
    write: suspend (Boolean) -> CanonicalWriteResult = { value ->
        CanonicalConfigRepository.commit(CanonicalMutation(listOf(CanonicalEdit.Toggle(field, value))))
    },
) {
    val state by CanonicalConfigRepository.state.collectAsState()
    val scope = rememberCoroutineScope()
    val checkWrite = LocalConfigWriteAccess.current
    var requested by remember { mutableStateOf<Boolean?>(null) }
    val pending = state.pending[field]
    PreferenceRowSwitch(
        title = title,
        subtitle =
            if (field == CanonicalToggle.DebugSwitch && state.activeCaptures > 0) {
                subtitle + "\n" + stringResource(R.string.config_capture_logging)
            } else {
                subtitle
            },
        icon = icon,
        index = index,
        count = count,
        checked = requested ?: pending ?: state.confirmed?.let { canonicalToggle(it, field) } ?: false,
        enabled = enabled && state.mode == ConfigCoordinatorMode.Open && requested == null && pending == null,
        progress = requested != null || pending != null,
        onDisabledClick =
            if (state.mode != ConfigCoordinatorMode.Open) {
                { checkWrite() }
            } else {
                null
            },
        onCheckedChange = change@{ value ->
            if (!checkWrite()) return@change
            requested = value
            scope.launch(start = CoroutineStart.UNDISPATCHED) {
                try {
                    write(value)
                } finally {
                    requested = null
                }
            }
        },
    )
}
