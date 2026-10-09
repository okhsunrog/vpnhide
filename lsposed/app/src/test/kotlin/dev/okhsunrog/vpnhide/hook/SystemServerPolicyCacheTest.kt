package dev.okhsunrog.vpnhide.hook

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class SystemServerPolicyCacheTest {
    private val first = SystemServerConfig(javaTargetHookMasksByAppId = mapOf(10_501 to 1L))
    private val second = SystemServerConfig(javaTargetHookMasksByAppId = mapOf(10_502 to 2L))

    private class Source {
        var now = 0L
        var fingerprint = SystemServerFingerprint(file(1), file(1))
        var config = SystemServerConfig()
        var reads = 0
        val publications = mutableListOf<SystemServerConfig>()
        val errors = mutableListOf<String?>()
        var afterRead: () -> Unit = {}
        var afterStat: () -> Unit = {}
        var failure: String? = null
        var statReadable = true
        var afterClock: () -> Unit = {}
        val cache =
            SystemServerPolicyCache(
                fingerprint = {
                    val result = fingerprint.takeIf { statReadable }
                    afterStat()
                    result
                },
                readConfig = {
                    reads++
                    val result = failure?.let { PolicyReadResult.Failed(it) } ?: PolicyReadResult.Loaded(config)
                    afterRead()
                    result
                },
                uptimeMillis = {
                    afterClock()
                    now
                },
                onLoaded = { publications += it },
                onReadError = { errors += it },
            )
    }

    @Test
    fun `unchanged sources reuse snapshot after polling interval`() {
        val source = Source().apply { config = first }
        assertEquals(first, source.cache.load())
        source.now = 1_000
        assertEquals(first, source.cache.load())
        assertEquals(1, source.reads)
    }

    @Test
    fun `canonical replacement during read never publishes mixed fingerprint`() {
        assertReplacementDuringRead { it.copy(config = file(2)) }
    }

    @Test
    fun `package replacement during read never publishes mixed fingerprint`() {
        assertReplacementDuringRead { it.copy(packagesList = file(2)) }
    }

    private fun assertReplacementDuringRead(replace: (SystemServerFingerprint) -> SystemServerFingerprint) {
        val source = Source().apply { config = first }
        source.afterRead = {
            source.config = second
            source.fingerprint = replace(source.fingerprint)
        }
        assertEquals(second, source.cache.load())
        assertEquals(listOf(second), source.publications)
        assertEquals(2, source.reads)
        assertEquals(second, source.cache.load())
        assertEquals(2, source.reads)
    }

    @Test
    fun `invalidation during reading forces another read with unchanged metadata`() {
        val source = Source().apply { config = first }
        source.afterRead = {
            if (source.reads == 1) {
                source.config = second
                source.cache.invalidate()
            }
        }
        assertEquals(second, source.cache.load())
        assertEquals(listOf(second), source.publications)
        assertEquals(2, source.reads)
    }

    @Test
    fun `invalidation just after publication is retained`() {
        var value = first
        var reads = 0
        lateinit var cache: SystemServerPolicyCache
        cache =
            SystemServerPolicyCache(
                fingerprint = { SystemServerFingerprint(file(1), file(1)) },
                readConfig = {
                    reads++
                    PolicyReadResult.Loaded(value)
                },
                uptimeMillis = { 0 },
                onLoaded = {
                    if (reads == 1) {
                        value = second
                        cache.invalidate()
                    }
                },
            )
        assertEquals(first, cache.load())
        assertEquals(second, cache.load())
        assertEquals(2, reads)
    }

    @Test
    fun `invalidation between stability check and publication cannot make stale entry fresh`() {
        val source = Source().apply { config = first }
        source.afterRead = {
            source.afterRead = {}
            source.afterClock = {
                source.afterClock = {}
                source.config = second
                source.cache.invalidate()
            }
        }
        assertEquals(first, source.cache.load())
        assertEquals(second, source.cache.load())
        assertEquals(2, source.reads)
    }

    @Test(timeout = 5_000)
    fun `watcher invalidation does not wait for reader lock`() {
        val source = Source().apply { config = first }
        source.afterRead = {
            source.afterRead = {}
            val watcher = Thread { source.cache.invalidate() }.apply { isDaemon = true }
            watcher.start()
            watcher.join(1_000)
            assertFalse("watcher must invalidate while the reader owns the lock", watcher.isAlive)
            source.config = second
        }
        assertEquals(second, source.cache.load())
        assertEquals(listOf(second), source.publications)
    }

    @Test
    fun `invalidation during unchanged stat prevents renewing old generation`() {
        val source = Source().apply { config = first }
        source.cache.load()
        source.now = 1_000
        source.afterStat = {
            source.afterStat = {}
            source.config = second
            source.cache.invalidate()
        }
        assertEquals(second, source.cache.load())
        assertEquals(2, source.reads)
    }

    @Test
    fun `missed watcher events for either source are repaired by polling`() {
        val source = Source().apply { config = first }
        source.cache.load()
        source.config = second
        source.fingerprint = source.fingerprint.copy(config = file(2))
        assertEquals(first, source.cache.load())
        source.now = 1_000
        assertEquals(second, source.cache.load())
        source.config = first
        source.fingerprint = source.fingerprint.copy(packagesList = file(2))
        source.now = 2_000
        assertEquals(first, source.cache.load())
        assertEquals(3, source.reads)
    }

    @Test
    fun `continuous changes return previous snapshot after two attempts then retry`() {
        val source = Source().apply { config = first }
        source.cache.load()
        source.config = second
        source.cache.invalidate()
        source.afterRead = {
            source.fingerprint = source.fingerprint.copy(config = file(source.reads.toLong()))
        }
        assertEquals(first, source.cache.load())
        assertEquals(3, source.reads)
        assertEquals(listOf(first), source.publications)
        source.afterRead = {}
        assertEquals(second, source.cache.load())
        assertEquals(4, source.reads)
    }

    @Test
    fun `continuous invalidations without initial snapshot fail open without caching`() {
        val source = Source().apply { config = first }
        source.afterRead = { source.cache.invalidate() }
        assertEquals(SystemServerConfig(), source.cache.load())
        assertEquals(2, source.reads)
        assertEquals(emptyList<SystemServerConfig>(), source.publications)
        source.afterRead = {}
        assertEquals(first, source.cache.load())
        assertEquals(3, source.reads)
    }

    @Test
    fun `stat failure is not cached as missing and recovers without watcher`() {
        val source = Source().apply { config = first }
        source.cache.load()
        source.now = 1_000
        source.statReadable = false
        assertEquals(first, source.cache.load())
        source.config = second
        source.fingerprint = source.fingerprint.copy(config = file(2))
        source.statReadable = true
        assertEquals(second, source.cache.load())
    }

    @Test
    fun `stat recovery with unchanged metadata rereads policy and clears error`() {
        val source = Source().apply { config = first }
        source.cache.load()
        source.now = 1_000
        source.statReadable = false
        assertEquals(first, source.cache.load())
        source.statReadable = true
        assertEquals(first, source.cache.load())
        assertEquals(2, source.reads)
        assertEquals(null, source.errors.last())
    }

    @Test
    fun `explicit empty policy and removed canonical source replace previous snapshot`() {
        val source = Source().apply { config = first }
        source.cache.load()
        source.config = SystemServerConfig()
        source.fingerprint = source.fingerprint.copy(config = null)
        source.cache.invalidate()
        assertEquals(SystemServerConfig(), source.cache.load())
        assertEquals(2, source.reads)
    }

    @Test
    fun `temporary read failure retains last good and retries unchanged source`() {
        val source = Source().apply { config = first }
        source.cache.load()
        source.failure = "malformed JSON"
        source.config = second
        source.cache.invalidate()
        assertEquals(first, source.cache.load())
        source.failure = null
        source.now = 1_000
        assertEquals(second, source.cache.load())
        assertEquals(3, source.reads)
        assertEquals(listOf(null, "malformed JSON", null), source.errors)
    }

    @Test
    fun `initial failure fails open then recovers`() {
        val source =
            Source().apply {
                config = first
                failure = "permission denied"
            }
        assertEquals(SystemServerConfig(), source.cache.load())
        source.failure = null
        source.now = 1_000
        assertEquals(first, source.cache.load())
    }

    private companion object {
        fun file(inode: Long): PolicyFileFingerprint = PolicyFileFingerprint(1, inode, 128, 1, 0, 1, 0)
    }
}
