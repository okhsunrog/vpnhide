package dev.okhsunrog.vpnhide.settings

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update

/** Process-owned DataStore subscription. Detaching a screen never publishes defaults. */
@OptIn(ExperimentalCoroutinesApi::class)
internal class SettingsStateOwner(
    source: Flow<AppSettings>,
    scope: CoroutineScope,
) {
    private val retries = MutableStateFlow(0)
    val state =
        retries
            .flatMapLatest {
                source.map { SettingsLoadState(settings = it) }.catch { error ->
                    if (error is CancellationException) throw error
                    emit(SettingsLoadState(error = error.javaClass.simpleName))
                }
            }.stateIn(scope, SharingStarted.Eagerly, SettingsLoadState())

    fun retry() {
        retries.update { it + 1 }
    }
}
