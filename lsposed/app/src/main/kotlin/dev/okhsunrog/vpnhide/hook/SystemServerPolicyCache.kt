package dev.okhsunrog.vpnhide.hook

import java.util.concurrent.atomic.AtomicLong

internal data class PolicyFileFingerprint(
    val device: Long,
    val inode: Long,
    val length: Long,
    val modifiedSeconds: Long,
    val modifiedNanos: Long,
    val changedSeconds: Long,
    val changedNanos: Long,
)

internal data class SystemServerFingerprint(
    val config: PolicyFileFingerprint?,
    val packagesList: PolicyFileFingerprint?,
)

internal sealed interface PolicyReadResult {
    data class Loaded(
        val config: SystemServerConfig,
    ) : PolicyReadResult

    data class Failed(
        val reason: String,
    ) : PolicyReadResult
}

/** Testable reader protocol; all Android I/O stays in SystemServerConfigCache. */
internal class SystemServerPolicyCache(
    private val fingerprint: () -> SystemServerFingerprint?,
    private val readConfig: () -> PolicyReadResult,
    private val uptimeMillis: () -> Long,
    private val onLoaded: (SystemServerConfig) -> Unit = {},
    private val onReadError: (String?) -> Unit = {},
) {
    private data class Cache(
        val generation: Long,
        val fingerprint: SystemServerFingerprint,
        val config: SystemServerConfig,
        val nextStatCheckUptimeMs: Long,
        val retryRead: Boolean = false,
    )

    private val generation = AtomicLong()
    private val lock = Any()

    @Volatile private var cache: Cache? = null

    fun invalidate() {
        // Never wait for a reader's I/O. Even a publication after this increment
        // carries the old generation and cannot satisfy another load.
        generation.incrementAndGet()
    }

    fun load(): SystemServerConfig {
        cache?.takeIf { it.isFresh(uptimeMillis()) }?.let { return it.config }
        return synchronized(lock) { loadLocked() }
    }

    private fun loadLocked(): SystemServerConfig {
        cache?.takeIf { it.isFresh(uptimeMillis()) }?.let { return it.config }
        repeat(MAX_READ_ATTEMPTS) {
            val readingGeneration = generation.get()
            val before = fingerprint() ?: return@repeat
            val previous = cache
            if (previous?.generation == readingGeneration && previous.fingerprint == before && !previous.retryRead) {
                if (generation.get() == readingGeneration) {
                    cache = previous.copy(nextStatCheckUptimeMs = nextStatCheck())
                    return previous.config
                }
                return@repeat
            }
            val result = readConfig()
            val after = fingerprint()
            if (before == after && generation.get() == readingGeneration) {
                val config =
                    when (result) {
                        is PolicyReadResult.Loaded -> result.config
                        is PolicyReadResult.Failed -> previous?.config ?: SystemServerConfig()
                    }
                cache = Cache(readingGeneration, before, config, nextStatCheck(), retryRead = result is PolicyReadResult.Failed)
                onReadError((result as? PolicyReadResult.Failed)?.reason)
                onLoaded(config)
                return config
            }
        }
        // Do not stamp an unstable read as current. The expired/invalidated
        // entry stays expired, so the next call retries without an inotify event.
        cache = cache?.copy(retryRead = true)
        onReadError("config sources unstable or unreadable; retry pending")
        return cache?.config ?: SystemServerConfig()
    }

    private fun Cache.isFresh(now: Long): Boolean =
        generation == this@SystemServerPolicyCache.generation.get() && now < nextStatCheckUptimeMs

    private fun nextStatCheck(): Long = uptimeMillis() + STAT_CHECK_INTERVAL_MS

    private companion object {
        const val STAT_CHECK_INTERVAL_MS = 1_000L
        const val MAX_READ_ATTEMPTS = 2
    }
}
