package dev.okhsunrog.vpnhide.core

import android.content.Context
import java.io.File

/**
 * Core module paths. The app reaches core over a socket core binds inside the app's own
 * device-encrypted data dir (owned by the app uid, labelled like the dir), so the app needs no
 * root and no other app can reach or probe it. Kept in sync with core-daemon `app_socket`
 * (`SOCKET_NAME`).
 */
object CorePaths {
    const val SOCKET_NAME = "vpnhide-core.sock"
    const val MODULE_DIR = "/data/adb/modules/vpnhide_core"

    /** The socket core serves this app on. Core binds it for the release package in user 0. */
    fun socket(context: Context): String {
        val dir = context.applicationInfo.deviceProtectedDataDir ?: context.createDeviceProtectedStorageContext().dataDir.absolutePath
        return File(dir, SOCKET_NAME).absolutePath
    }
}
