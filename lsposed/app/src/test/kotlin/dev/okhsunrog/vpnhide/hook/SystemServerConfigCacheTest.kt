package dev.okhsunrog.vpnhide.hook

import org.junit.Assert.assertEquals
import org.junit.Test

class SystemServerConfigCacheTest {
    @Test
    fun `hidden package is visible to caller with same app id`() {
        val config =
            SystemServerConfig(
                hiddenPackages = setOf("ru.vk.store", "com.example.vpn"),
                packageAppIds =
                    mapOf(
                        "ru.vk.store" to 10_501,
                        "com.example.vpn" to 10_777,
                    ),
            )

        assertEquals(false, config.shouldHidePackageForCallerAppId("ru.vk.store", callerAppId = 10_501))
        assertEquals(true, config.shouldHidePackageForCallerAppId("ru.vk.store", callerAppId = 10_777))
        assertEquals(false, config.shouldHidePackageForCallerAppId("com.not.hidden", callerAppId = 10_777))
    }

    @Test
    fun `hidden package without resolved app id stays hidden`() {
        val config = SystemServerConfig(hiddenPackages = setOf("com.missing"))

        assertEquals(true, config.shouldHidePackageForCallerAppId("com.missing", callerAppId = 10_501))
    }
}
