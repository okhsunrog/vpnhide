package dev.okhsunrog.vpnhide.settings

import org.junit.Assert.assertEquals
import org.junit.Test

class AppLanguageDataTest {
    @Test
    fun systemChoiceUsesAnEmptyLocaleList() {
        assertEquals("", AppLanguage.System.tag)
        assertEquals(AppLanguage.System, appLanguageForTag(""))
    }

    @Test
    fun regionalAndOrderedSystemSelectionsMatchThePicker() {
        assertEquals(AppLanguage.English, appLanguageForTag("en-US"))
        assertEquals(AppLanguage.Russian, appLanguageForTag("ru-RU,en"))
        assertEquals(AppLanguage.Chinese, appLanguageForTag("zh-Hans-CN"))
        assertEquals(AppLanguage.English, appLanguageForTag("en,ru"))
    }

    @Test
    fun unsupportedSelectionFallsBackToSystem() {
        assertEquals(AppLanguage.System, appLanguageForTag("de-DE"))
    }

    @Test
    fun everyExplicitChoiceRoundTrips() {
        AppLanguage.entries.forEach { assertEquals(it, appLanguageForTag(it.tag)) }
    }
}
