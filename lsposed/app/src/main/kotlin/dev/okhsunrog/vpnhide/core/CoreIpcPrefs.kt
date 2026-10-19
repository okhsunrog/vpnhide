package dev.okhsunrog.vpnhide.core

import android.content.Context

/**
 * Local preferences for the core IPC client path. Editor drafts stay elsewhere in Kotlin;
 * this only gates whether production reads/writes prefer the core replica over the legacy
 * su root-mutation / root-snapshot lane.
 *
 * Default **false**: legacy [dev.okhsunrog.vpnhide.ConfigRootIo] remains authoritative until
 * device validation completes. Flip via debug settings or:
 * `adb shell ... settings` / SharedPreferences `vpnhide_core_ipc`.
 */
object CoreIpcPrefs {
    private const val PREFS = "vpnhide_core_ipc"
    private const val KEY_USE_CORE = "use_core_replica"

    fun useCoreReplica(context: Context): Boolean =
        context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getBoolean(KEY_USE_CORE, false)

    fun setUseCoreReplica(
        context: Context,
        enabled: Boolean,
    ) {
        context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .putBoolean(KEY_USE_CORE, enabled)
            .apply()
    }
}
