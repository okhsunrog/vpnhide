package dev.okhsunrog.vpnhide.hook

import dev.okhsunrog.vpnhide.CanonicalApp
import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.LsposedJavaHookEntries
import dev.okhsunrog.vpnhide.bit
import org.junit.Assert.assertEquals
import org.junit.Test

class SystemServerConfigDataTest {
    private val first = LsposedJavaHookEntries[0]
    private val second = LsposedJavaHookEntries[1]
    private val ids = mapOf("first" to 10_501, "second" to 10_501)

    @Test
    fun `shared app id unions enabled masks independent of package order`() {
        val apps = linkedMapOf("first" to selection(first.hookName), "second" to selection(second.hookName))
        val expected = mapOf(10_501 to (first.bit or second.bit))
        assertEquals(expected, javaHookMasksByAppId(CanonicalConfig(apps = apps), ids))
        assertEquals(expected, javaHookMasksByAppId(CanonicalConfig(apps = apps.entries.reversed().associate { it.toPair() }), ids))
    }

    @Test
    fun `disabled and empty roles cannot veto enabled shared package`() {
        for (disabled in listOf(
            CanonicalApp(),
            CanonicalApp(javaHooks = listOf(second.hookName)),
            CanonicalApp(java = true, javaHooks = emptyList()),
        )) {
            val config = CanonicalConfig(apps = mapOf("first" to selection(first.hookName), "second" to disabled))
            assertEquals(mapOf(10_501 to first.bit), javaHookMasksByAppId(config, ids))
        }
    }

    @Test
    fun `identical shared selections do not add bits`() {
        val config = CanonicalConfig(apps = mapOf("first" to selection(first.hookName), "second" to selection(first.hookName)))
        assertEquals(mapOf(10_501 to first.bit), javaHookMasksByAppId(config, ids))
    }

    @Test
    fun `disabled unresolved and unknown hooks create no target`() {
        val config =
            CanonicalConfig(
                apps =
                    mapOf(
                        "first" to CanonicalApp(),
                        "second" to selection("unknown"),
                        "missing" to CanonicalApp(java = true),
                    ),
            )
        assertEquals(emptyMap<Int, Long>(), javaHookMasksByAppId(config, ids))
    }

    @Test
    fun `different app ids stay separate and same app id matches across profiles`() {
        val config = CanonicalConfig(apps = mapOf("first" to selection(first.hookName), "second" to selection(second.hookName)))
        val masks = javaHookMasksByAppId(config, ids + ("second" to 10_502))
        assertEquals(mapOf(10_501 to first.bit, 10_502 to second.bit), masks)
        assertEquals(masks[SystemServerConfigCache.appId(10_501)], masks[SystemServerConfigCache.appId(210_501)])
    }

    @Test
    fun `default enabled role contributes every Java hook`() {
        val config = CanonicalConfig(apps = mapOf("first" to selection(first.hookName), "second" to CanonicalApp(java = true)))
        assertEquals(mapOf(10_501 to LsposedJavaHookEntries.fold(0L) { mask, hook -> mask or hook.bit }), javaHookMasksByAppId(config, ids))
    }

    private fun selection(name: String): CanonicalApp = CanonicalApp(java = true, javaHooks = listOf(name))
}
