package dev.okhsunrog.vpnhide.settings

import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.appcompat.app.AppCompatActivity
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.picker.TargetActionBar
import dev.okhsunrog.vpnhide.ui.components.MetricContent
import dev.okhsunrog.vpnhide.ui.components.MetricGrid
import dev.okhsunrog.vpnhide.ui.components.ModuleHeading
import dev.okhsunrog.vpnhide.ui.theme.VpnHideTheme

class LayoutHarness : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            CompositionLocalProvider(LocalSettingsState provides AppSettings(themeMode = ThemeMode.Dark)) {
                VpnHideTheme {
                    Surface(Modifier.fillMaxSize()) {
                        if (intent.getBooleanExtra("metrics", false)) {
                            Column(
                                Modifier.padding(horizontal = 34.dp, vertical = 16.dp).verticalScroll(rememberScrollState()),
                                verticalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Text("VPN выключен", style = MaterialTheme.typography.headlineSmall)
                                MetricGrid(
                                    MetricContent("Модули", "3/3", MaterialTheme.colorScheme.primary),
                                    MetricContent("Нативный уровень", "Не проверено", MaterialTheme.colorScheme.onSurface),
                                    MetricContent("Java API уровень", "Не проверено", MaterialTheme.colorScheme.onSurface),
                                    MetricContent("Проблемы", "0", MaterialTheme.colorScheme.primary),
                                )
                                Row {
                                    Spacer(Modifier.width(72.dp))
                                    Box(Modifier.weight(1f)) { ModuleHeading("Модуль скрытия портов", "1.3.0") }
                                    Spacer(Modifier.width(19.dp))
                                }
                            }
                        } else {
                            Column(Modifier.fillMaxSize().systemBarsPadding()) {
                                Text("VPN Hide · Скрытие", modifier = Modifier.padding(16.dp), style = MaterialTheme.typography.titleLarge)
                                Box(Modifier.weight(1f).fillMaxWidth()) { Text("Место списка приложений", Modifier.padding(16.dp)) }
                                Surface(tonalElevation = 3.dp) {
                                    TargetActionBar(
                                        count =
                                            if (intent.getBooleanExtra(
                                                    "long_count",
                                                    false,
                                                )
                                            ) {
                                                "Скрыто: 3; автоматически: 3; вручную: 0"
                                            } else {
                                                stringResource(R.string.selected_count, 56)
                                            },
                                        supportingText = if (intent.getBooleanExtra("long_count", false)) "Исключено: 2" else null,
                                        canDiscard = false,
                                        canSave = false,
                                        saving = false,
                                        onDiscard = {},
                                        onSave = {},
                                    )
                                }
                                Text("Обзор       Скрытие       Статистика", Modifier.padding(16.dp))
                            }
                        }
                    }
                }
            }
        }
    }
}
