package dev.okhsunrog.vpnhide.picker

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.StatusBanner
import dev.okhsunrog.vpnhide.StatusColors
import dev.okhsunrog.vpnhide.diagnostics.DetectionMethod
import dev.okhsunrog.vpnhide.generated.HookIds

@Composable
internal fun HooksDialog(
    app: AppEntry,
    title: String,
    hookEntries: List<HookIds.Hook>,
    selectedHooks: List<String>?,
    roleEnabled: Boolean,
    notice: String? = null,
    onDismiss: () -> Unit,
    onSave: (List<String>?) -> Unit,
) {
    val hookNames = remember(hookEntries) { hookEntries.map { it.hookName } }
    // `selectedHooks == null` is ambiguous: it means BOTH "role on with all
    // hooks" and "role off". Disambiguate with roleEnabled so opening the dialog
    // on a disabled role starts with nothing checked — otherwise it would show
    // every hook pre-checked and Save would silently turn the role into a full
    // target (resolveHookSelection(all) -> null -> enabled).
    var selected by remember(app.packageName, selectedHooks, roleEnabled, hookNames) {
        mutableStateOf(
            when {
                selectedHooks != null -> selectedHooks.toSet()
                roleEnabled -> hookNames.toSet()
                else -> emptySet()
            },
        )
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    text = app.label,
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(
                    text = app.packageName,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                notice?.let {
                    StatusBanner(
                        text = it,
                        containerColor = StatusColors.warningContainer(),
                        contentColor = MaterialTheme.colorScheme.onSurface,
                    )
                }
                val grouped =
                    remember(hookEntries) {
                        hookEntries.groupBy { DetectionMethod.of(it) }.toSortedMap(compareBy { it.ordinal })
                    }
                LazyColumn(modifier = Modifier.heightIn(max = 460.dp)) {
                    grouped.forEach { (method, hooks) ->
                        item(key = "header_${method.name}") {
                            MethodHeader(method)
                        }
                        items(hooks, key = { it.hookName }) { hook ->
                            HookRow(
                                hook = hook,
                                checked = hook.hookName in selected,
                                onCheckedChange = { enabled ->
                                    selected =
                                        if (enabled) {
                                            selected + hook.hookName
                                        } else {
                                            selected - hook.hookName
                                        }
                                },
                            )
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onSave(resolveHookSelection(hookNames, selected)) }) {
                Text(stringResource(R.string.btn_save))
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text(stringResource(R.string.btn_cancel))
            }
        },
    )
}

@Composable
private fun MethodHeader(method: DetectionMethod) {
    Column(modifier = Modifier.padding(top = 10.dp, bottom = 2.dp)) {
        Text(
            text = stringResource(method.labelRes),
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.Bold,
        )
        Text(
            text = stringResource(method.descriptionRes),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun HookRow(
    hook: HookIds.Hook,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    Row(
        modifier =
            Modifier
                .clickable { onCheckedChange(!checked) }
                .fillMaxWidth()
                .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = checked, onCheckedChange = onCheckedChange)
        Text(
            text = hook.note,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(start = 8.dp),
        )
    }
}
