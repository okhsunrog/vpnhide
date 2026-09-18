package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.settings.AppSettings
import dev.okhsunrog.vpnhide.settings.SettingsRepository
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * The developer switches, as a flow the Dashboard projection can follow.
 *
 * They used to be read inside `deriveEnvironmentFacts`, i.e. baked into
 * [DashboardRootFacts], which are cached until something invalidates them
 * (refresh, Retry, a Save, a root change). Writing a DataStore preference
 * invalidates none of that, so flipping a toggle changed nothing on screen until
 * the next refresh or app restart — the flag looked broken.
 *
 * Here they are their own source instead: [DashboardCache.state] combines this
 * flow with the facts, so a toggle recomposes the banners immediately and costs
 * no root read at all. The collection starts from [DashboardCache]'s own load,
 * which is the first place in the process with both a context and a reason to
 * care, so nothing depends on an Activity having run first.
 */
internal object DeveloperFlagsCache {
    private val state = MutableStateFlow(DeveloperFlags())

    /** Defaults until DataStore's first emission — a few milliseconds, long before root facts exist. */
    val flags: StateFlow<DeveloperFlags> = state.asStateFlow()

    private var started = false

    @Synchronized
    fun start(context: Context) {
        if (started) return
        started = true
        val repository = SettingsRepository(context.applicationContext)
        ObservationRuntime.scope.launch {
            repository.settings.collect { settings ->
                state.value = settings.toDeveloperFlags()
            }
        }
    }
}

/** The one place the preferences are read as Dashboard input. */
internal fun AppSettings.toDeveloperFlags() =
    DeveloperFlags(
        agentBridgeOn = agentControlEnabled,
        ignoreVersionMismatch = ignoreVersionMismatch,
    )
