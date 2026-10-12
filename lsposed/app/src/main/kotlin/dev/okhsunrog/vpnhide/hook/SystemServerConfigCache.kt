package dev.okhsunrog.vpnhide.hook

import android.os.SystemClock
import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import dev.okhsunrog.vpnhide.CANONICAL_CONFIG_FILE
import dev.okhsunrog.vpnhide.LsposedStats
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.hasHook
import dev.okhsunrog.vpnhide.parseCanonicalConfig
import java.io.File

internal data class SystemServerConfig(
    val javaTargetHookMasksByAppId: Map<Int, Long> = emptyMap(),
    val observerAppIds: Set<Int> = emptySet(),
    val hiddenPackages: Set<String> = emptySet(),
    val packageAppIds: Map<String, Int> = emptyMap(),
    val debug: Boolean = false,
) {
    val javaTargetAppIds: Set<Int>
        get() = javaTargetHookMasksByAppId.keys

    fun shouldHidePackageForCallerAppId(
        packageName: String,
        callerAppId: Int,
    ): Boolean {
        if (packageName !in hiddenPackages) return false
        return packageAppIds[packageName] != callerAppId
    }
}

/**
 * Canonical config reader for hooks running inside system_server.
 *
 * It resolves package names through `/data/system/packages.list`, not
 * PackageManager, because these hooks run in PackageManager/ConnectivityManager
 * call paths. Hook matching uses `callingUid % 100000` so every Android profile
 * for the same appId is covered.
 */
internal object SystemServerConfigCache {
    private const val USER_ID_MODULO = 100_000
    private val configFile = File(CANONICAL_CONFIG_FILE)
    private val packagesListFile = File("/data/system/packages.list")

    private val policyCache =
        SystemServerPolicyCache(
            fingerprint = ::fingerprint,
            readConfig = ::readConfig,
            uptimeMillis = SystemClock::uptimeMillis,
            onReadError = LsposedStats::setPolicyReadError,
            onLoaded = { result ->
                // Poll both sources even if the FileObserver misses atomic renames.
                HookLog.enabled = result.debug
                HookLog.i(
                    "VpnHide: system_server config loaded " +
                        "java=${result.javaTargetAppIds.size} observer=${result.observerAppIds.size} " +
                        "hidden=${result.hiddenPackages.size} debug=${result.debug}",
                )
            },
        )

    fun load(): SystemServerConfig = policyCache.load()

    fun invalidate() = policyCache.invalidate()

    fun isTargetUid(uid: Int): Boolean {
        if (uid < android.os.Process.FIRST_APPLICATION_UID) return false
        return load().javaTargetAppIds.contains(appId(uid))
    }

    fun isHookEnabledForUid(
        uid: Int,
        hook: HookIds.Hook,
    ): Boolean {
        if (uid < android.os.Process.FIRST_APPLICATION_UID) return false
        return load().javaTargetHookMasksByAppId[appId(uid)]?.hasHook(hook) == true
    }

    fun appId(uid: Int): Int = uid % USER_ID_MODULO

    private fun readConfig(): PolicyReadResult =
        try {
            if (!configFile.exists()) return PolicyReadResult.Loaded(SystemServerConfig())
            val canonical =
                parseCanonicalConfig(configFile.readText())
                    ?: return PolicyReadResult.Failed("empty or unsupported canonical config")
            val packageAppIds = parsePackagesListAppIds(packagesListFile.takeIf(File::isFile)?.readText().orEmpty())
            val observers =
                canonical.apps
                    .filterValues { it.appHiding }
                    .keys
                    .resolveAppIds(packageAppIds)
            val hidden = canonical.apps.filterValues { it.hidden }.keys
            PolicyReadResult.Loaded(
                SystemServerConfig(
                    javaTargetHookMasksByAppId = javaHookMasksByAppId(canonical, packageAppIds),
                    observerAppIds = observers,
                    hiddenPackages = hidden,
                    packageAppIds = packageAppIds,
                    debug = canonical.debug,
                ),
            )
        } catch (t: Throwable) {
            HookLog.e("VpnHide: failed to read canonical config: ${t.message}")
            PolicyReadResult.Failed(t.message ?: t.javaClass.simpleName)
        }

    private fun Set<String>.resolveAppIds(packageAppIds: Map<String, Int>): Set<Int> = mapNotNull(packageAppIds::get).toSet()

    private fun fingerprint(): SystemServerFingerprint? =
        try {
            SystemServerFingerprint(configFile.fingerprint(), packagesListFile.fingerprint())
        } catch (e: ErrnoException) {
            HookLog.e("VpnHide: failed to stat config sources: ${e.message}")
            null // An unreadable source must never look like a stable missing file.
        }

    private fun File.fingerprint(): PolicyFileFingerprint? =
        try {
            val stat = Os.stat(path)
            PolicyFileFingerprint(
                device = stat.st_dev,
                inode = stat.st_ino,
                length = stat.st_size,
                modifiedSeconds = stat.st_mtim.tv_sec,
                modifiedNanos = stat.st_mtim.tv_nsec,
                changedSeconds = stat.st_ctim.tv_sec,
                changedNanos = stat.st_ctim.tv_nsec,
            )
        } catch (e: ErrnoException) {
            if (e.errno != OsConstants.ENOENT) throw e
            null
        }
}

internal fun parsePackagesListAppIds(raw: String): Map<String, Int> =
    raw
        .lineSequence()
        .mapNotNull { line ->
            val parts = line.trim().split(Regex("\\s+"), limit = 3)
            val pkg = parts.getOrNull(0)?.takeIf { it.isNotBlank() } ?: return@mapNotNull null
            val appId = parts.getOrNull(1)?.toIntOrNull() ?: return@mapNotNull null
            pkg to appId
        }.toMap()
