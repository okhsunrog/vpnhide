package dev.okhsunrog.vpnhide.hook

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test

class NetworkCapabilitiesCompatDataTest {
    private val network = Any()

    @Test
    fun android11UsesPackageAttributedAccessor() {
        val calls = mutableListOf<List<Any?>>()
        val vpnCapabilities = Any()
        val result =
            readNetworkCapabilitiesCompat(network) { args ->
                calls += args.toList()
                if (args.size != 2) throw NoSuchMethodError("getNetworkCapabilities")
                vpnCapabilities
            }
        assertSame(vpnCapabilities, result)
        assertEquals(listOf(listOf(network, "android", null), listOf(network, "android")), calls)
    }

    @Test
    fun modernAccessorKeepsAttributionTagAndStops() {
        val calls = mutableListOf<List<Any?>>()
        assertEquals(
            "vpn",
            readNetworkCapabilitiesCompat(network) { args ->
                calls += args.toList()
                "vpn"
            },
        )
        assertEquals(listOf(listOf(network, "android", null)), calls)
    }

    @Test
    fun legacyAccessorStillWorks() {
        val calls = mutableListOf<List<Any?>>()
        assertEquals(
            "vpn",
            readNetworkCapabilitiesCompat(network) { args ->
                calls += args.toList()
                if (args.size != 1) throw NoSuchMethodException("getNetworkCapabilities")
                "vpn"
            },
        )
        assertEquals(listOf(listOf(network, "android", null), listOf(network, "android"), listOf(network)), calls)
    }

    @Test
    fun disappearedNetworkDoesNotRetryAnOlderAccessor() {
        var calls = 0
        assertNull(
            readNetworkCapabilitiesCompat<Any>(network) {
                calls++
                null
            },
        )
        assertEquals(1, calls)
    }

    @Test
    fun missingAllSignaturesReturnsUnknown() {
        var calls = 0
        assertNull(
            readNetworkCapabilitiesCompat<Any>(network) {
                calls++
                throw NoSuchMethodError()
            },
        )
        assertEquals(3, calls)
    }

    @Test(expected = SecurityException::class)
    fun permissionFailureDoesNotRetryWithLessAttribution() {
        readNetworkCapabilitiesCompat<Any>(network) { throw SecurityException("denied") }
    }
}
