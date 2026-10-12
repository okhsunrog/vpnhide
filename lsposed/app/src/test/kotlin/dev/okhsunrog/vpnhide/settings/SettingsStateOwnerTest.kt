package dev.okhsunrog.vpnhide.settings

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.io.IOException

@OptIn(ExperimentalCoroutinesApi::class)
class SettingsStateOwnerTest {
    @Test
    fun delayedReadNeverPublishesDefaultPreferences() =
        runTest {
            val source = MutableSharedFlow<AppSettings>()
            val owner = SettingsStateOwner(source, backgroundScope)
            val sideEffects = mutableListOf<Boolean>()
            backgroundScope.launch { owner.state.collect { it.settings?.let { settings -> sideEffects += settings.agentControlEnabled } } }
            runCurrent()
            assertNull(owner.state.value.settings)
            assertEquals(emptyList<Boolean>(), sideEffects)
            source.emit(AppSettings(cornerStyle = CornerStyle.Rounded, agentControlEnabled = true))
            runCurrent()
            assertEquals(
                CornerStyle.Rounded,
                owner.state.value.settings
                    ?.cornerStyle,
            )
            assertEquals(listOf(true), sideEffects)
        }

    @Test
    fun recreatedCollectorStartsWithSavedValuesAndNeverDisablesFeatures() =
        runTest {
            val source = MutableSharedFlow<AppSettings>()
            val owner = SettingsStateOwner(source, backgroundScope)
            val saved = AppSettings(cornerStyle = CornerStyle.Rounded, themeMode = ThemeMode.Dark, agentControlEnabled = true)
            runCurrent()
            source.emit(saved)
            runCurrent()
            repeat(3) {
                assertEquals(saved, owner.state.value.settings)
                val seen = mutableListOf<SettingsLoadState>()
                val screen = backgroundScope.launch { owner.state.collect { seen += it } }
                runCurrent()
                screen.cancel()
                assertEquals(listOf(SettingsLoadState(saved)), seen)
            }
        }

    @Test
    fun updatesWhileNoScreenIsAttachedAreRetained() =
        runTest {
            val source = MutableSharedFlow<AppSettings>()
            val owner = SettingsStateOwner(source, backgroundScope)
            runCurrent()
            source.emit(AppSettings(cornerStyle = CornerStyle.Rounded))
            runCurrent()
            val latest = AppSettings(cornerStyle = CornerStyle.Smooth, amoled = true)
            source.emit(latest)
            runCurrent()
            assertEquals(latest, owner.state.value.settings)
            assertEquals(1, source.subscriptionCount.value)
        }

    @Test
    fun readFailureIsExplicitAndRetryLoadsRealPreferences() =
        runTest {
            var attempts = 0
            val saved = AppSettings(animationsEnabled = false)
            val source =
                flow {
                    if (attempts++ == 0) throw IOException("unavailable")
                    emit(saved)
                }
            val owner = SettingsStateOwner(source, backgroundScope)
            runCurrent()
            assertNull(owner.state.value.settings)
            assertEquals("IOException", owner.state.value.error)
            owner.retry()
            runCurrent()
            assertEquals(SettingsLoadState(saved), owner.state.value)
        }
}
