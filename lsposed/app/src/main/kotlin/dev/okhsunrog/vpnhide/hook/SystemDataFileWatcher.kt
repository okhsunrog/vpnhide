package dev.okhsunrog.vpnhide.hook

import android.os.FileObserver

// String path, not File: FileObserver(File, Int) is API 29+, while the
// FileObserver(String, Int) form works back to API 28 (Android 9).
private const val SYSTEM_DATA_DIR = "/data/system"

/**
 * Watch `/data/system` for the events our config writers produce and invoke
 * [onChange] with the affected filename. Shared by the three system_server
 * LSPosed watchers (HookEntry's UID cache, PackageVisibilityHooks' hidden /
 * observer caches, HookLog's debug-logging flag) — each reacts to a different
 * file, so the per-file dispatch stays in the caller's [onChange] lambda.
 *
 * CLOSE_WRITE + MOVED_TO catch our writers (a single `> file` redirect or an
 * atomic `mv`) only once they finish. MODIFY would fire mid-write and let a
 * reader see a half-populated file, so it is opt-in via [extraEvents] — only
 * HookLog enables it, where a transient partial read of a one-byte flag is
 * harmless.
 *
 * Returns the already-started observer; the caller holds the reference so it
 * isn't garbage-collected (which would silently stop the watch).
 */
internal fun watchSystemDataDir(
    extraEvents: Int = 0,
    onChange: (filename: String) -> Unit,
): FileObserver {
    val observer =
        @Suppress("DEPRECATION")
        object : FileObserver(
            SYSTEM_DATA_DIR,
            CREATE or CLOSE_WRITE or MOVED_TO or extraEvents,
        ) {
            override fun onEvent(
                event: Int,
                path: String?,
            ) {
                if (path != null) onChange(path)
            }
        }
    observer.startWatching()
    return observer
}
