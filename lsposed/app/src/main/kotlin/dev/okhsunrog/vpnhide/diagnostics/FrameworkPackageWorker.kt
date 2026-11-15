@file:Suppress("DEPRECATION") // Int flags also cover the app's pre-API-33 devices.

package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import android.content.pm.PackageManager
import android.os.Process
import uniffi.vpnhide_core_ffi.CoreFrameworkPhase
import uniffi.vpnhide_core_ffi.CoreFrameworkTask
import uniffi.vpnhide_core_ffi.CorePackageObservation
import uniffi.vpnhide_core_ffi.CorePackageQuery
import java.security.MessageDigest

internal fun observeFrameworkPackages(
    context: Context,
    task: CoreFrameworkTask,
): List<CorePackageObservation> {
    val target = task.packageVisibility ?: return emptyList()
    val pm = context.packageManager
    val ownPackage = context.packageName
    // PackageManager's process-local query caches include the flags in their
    // key. Use a fresh key per phase without invalidating the device's cache.
    val flags =
        when (task.phase) {
            CoreFrameworkPhase.PREPARED -> 0
            CoreFrameworkPhase.RAW -> PackageManager.GET_META_DATA
            CoreFrameworkPhase.PROTECTED -> PackageManager.GET_SHARED_LIBRARY_FILES
            CoreFrameworkPhase.REVOKED -> PackageManager.GET_META_DATA or PackageManager.GET_SHARED_LIBRARY_FILES
            CoreFrameworkPhase.FINISHED -> error("no package task after completion")
        }
    return CorePackageQuery.entries.mapNotNull { query ->
        runCatching {
            when (query) {
                CorePackageQuery.PACKAGE_INFO -> {
                    lookup(query, target, ownPackage) { name ->
                        val info = pm.getPackageInfo(name, flags)
                        require(info.packageName == name)
                        requireNotNull(info.applicationInfo).uid
                    }
                }

                CorePackageQuery.APPLICATION_INFO -> {
                    lookup(query, target, ownPackage) { name ->
                        val info = pm.getApplicationInfo(name, flags)
                        require(info.packageName == name)
                        info.uid
                    }
                }

                CorePackageQuery.INSTALLED_PACKAGES -> {
                    enumerate(query, target, ownPackage, pm.getInstalledPackages(0).map { it.packageName })
                }

                CorePackageQuery.INSTALLED_APPLICATIONS -> {
                    enumerate(query, target, ownPackage, pm.getInstalledApplications(0).map { it.packageName })
                }
            }
        }.getOrNull()
    }
}

private inline fun lookup(
    query: CorePackageQuery,
    target: String,
    ownPackage: String,
    uid: (String) -> Int,
): CorePackageObservation {
    val present =
        try {
            require(uid(target) >= 0)
            true
        } catch (_: PackageManager.NameNotFoundException) {
            false
        }
    val ordinary = runCatching { uid(ownPackage) == Process.myUid() }.getOrDefault(false)
    return CorePackageObservation(query, present, ordinary, null)
}

private fun enumerate(
    query: CorePackageQuery,
    target: String,
    ownPackage: String,
    packages: List<String>,
): CorePackageObservation {
    val remaining = packages.filterNot { it == target }.sorted().joinToString("\u0000")
    val sha256 =
        MessageDigest.getInstance("SHA-256").digest(remaining.toByteArray()).joinToString("") { "%02x".format(it) }
    return CorePackageObservation(query, target in packages, ownPackage in packages, sha256)
}
