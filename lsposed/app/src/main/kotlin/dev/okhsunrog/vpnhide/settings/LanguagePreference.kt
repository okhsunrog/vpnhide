package dev.okhsunrog.vpnhide.settings

import androidx.appcompat.app.AppCompatDelegate
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Language
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.ui.components.PreferenceRow

@Composable
internal fun LanguagePreference(
    index: Int,
    count: Int,
) {
    val interactor = LocalSettingsInteractor.current
    // Observe configuration so returning from the system language picker updates this row too.
    var selected by remember { mutableStateOf(appLanguageForTag(AppCompatDelegate.getApplicationLocales().toLanguageTags())) }
    LifecycleResumeEffect(Unit) {
        selected = appLanguageForTag(AppCompatDelegate.getApplicationLocales().toLanguageTags())
        onPauseOrDispose { }
    }
    var open by rememberSaveable { mutableStateOf(false) }
    val systemLabel = stringResource(R.string.language_system)
    PreferenceRow(
        title = stringResource(R.string.settings_language),
        subtitle = if (selected == AppLanguage.System) systemLabel else selected.nativeName,
        icon = Icons.Default.Language,
        index = index,
        count = count,
        onClick = { open = true },
    )
    if (open) {
        AlertDialog(
            onDismissRequest = { open = false },
            title = { Text(stringResource(R.string.settings_language)) },
            text = {
                Column(Modifier.selectableGroup()) {
                    AppLanguage.entries.forEach { language ->
                        Row(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .selectable(
                                        selected = selected == language,
                                        role = Role.RadioButton,
                                        onClick = {
                                            open = false
                                            selected = language
                                            interactor.setAppLanguage(language.tag)
                                        },
                                    ).padding(vertical = 12.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = selected == language, onClick = null)
                            Text(
                                if (language == AppLanguage.System) systemLabel else language.nativeName,
                                modifier = Modifier.padding(start = 12.dp),
                            )
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { open = false }) { Text(stringResource(android.R.string.cancel)) }
            },
        )
    }
}
