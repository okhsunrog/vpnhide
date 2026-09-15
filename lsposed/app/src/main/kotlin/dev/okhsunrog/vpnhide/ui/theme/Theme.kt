package dev.okhsunrog.vpnhide.ui.theme

import android.app.Activity
import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat
import com.materialkolor.PaletteStyle
import com.materialkolor.rememberDynamicColorScheme
import dev.okhsunrog.vpnhide.settings.AppSettings
import dev.okhsunrog.vpnhide.settings.LocalSettingsState
import dev.okhsunrog.vpnhide.settings.ThemeMode

/**
 * The resolved dark/light state — honours the in-app [ThemeMode] override, not
 * just the system setting. Read this (never `isSystemInDarkTheme()`) for any
 * theme-aware colour outside the Material colorScheme, e.g. the pinned
 * [dev.okhsunrog.vpnhide.StatusColors]; otherwise a manual Dark override drifts
 * to light banners/icons when the system is Light.
 */
val LocalDarkTheme = staticCompositionLocalOf { false }

/**
 * Root theme for the picker app.
 *
 * Reads the live [AppSettings] from [LocalSettingsState] and builds an
 * expressive Material 3 theme: wallpaper-derived Material You on Android 12+
 * (or a material-kolor scheme generated from the brand seed otherwise), with
 * AMOLED, contrast and motion honoring the user's preferences.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun VpnHideTheme(content: @Composable () -> Unit) {
    val settings = LocalSettingsState.current
    val dark = settings.themeMode.isDark()

    val colorScheme = rememberAppColorScheme(settings, dark)

    // Keep the system bars' icon appearance in sync with the resolved theme, so a
    // manual Dark override still gets light bar icons when the system is Light.
    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as Activity).window
            val controller = WindowCompat.getInsetsController(window, view)
            controller.isAppearanceLightStatusBars = !dark
            controller.isAppearanceLightNavigationBars = !dark
        }
    }

    CompositionLocalProvider(LocalDarkTheme provides dark) {
        MaterialExpressiveTheme(
            colorScheme = colorScheme,
            motionScheme = AppMotionScheme,
            shapes = appShapes(settings.cornerStyle),
            content = content,
        )
    }
}

@Composable
private fun ThemeMode.isDark(): Boolean =
    when (this) {
        ThemeMode.System -> isSystemInDarkTheme()
        ThemeMode.Light -> false
        ThemeMode.Dark -> true
    }

@Composable
private fun rememberAppColorScheme(
    settings: AppSettings,
    dark: Boolean,
): ColorScheme {
    val context = LocalContext.current
    val supportsDynamic = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S

    val base =
        if (settings.dynamicColor && supportsDynamic) {
            if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        } else if (settings.dynamicColor) {
            // Pre-12 fallback when "Material You" is on but unavailable.
            if (dark) darkColorScheme() else lightColorScheme()
        } else {
            rememberDynamicColorScheme(
                seedColor = Color(settings.seedColor),
                isDark = dark,
                isAmoled = settings.amoled,
                style = PaletteStyle.TonalSpot,
                contrastLevel = settings.contrast.toDouble(),
            )
        }

    // The material-kolor path already bakes in AMOLED; the system/dynamic path
    // does not, so apply it here for dark mode.
    return if (settings.amoled && dark && (settings.dynamicColor)) base.toAmoled() else base
}
