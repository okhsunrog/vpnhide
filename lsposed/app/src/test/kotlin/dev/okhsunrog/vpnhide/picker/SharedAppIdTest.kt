package dev.okhsunrog.vpnhide.picker

import org.junit.Assert.assertEquals
import org.junit.Test

class SharedAppIdTest {
    @Test
    fun `distinct packages sharing app id across profiles are marked`() {
        assertEquals(
            setOf("first", "second"),
            packagesSharingAppIds(mapOf("first" to listOf(10_501, 110_501), "second" to listOf(210_501), "other" to listOf(10_502))),
        )
    }

    @Test
    fun `duplicate uids and multiple profiles of one package are not sharing`() {
        assertEquals(emptySet<String>(), packagesSharingAppIds(mapOf("first" to listOf(10_501, 10_501, 110_501), "missing" to emptyList())))
    }

    @Test
    fun `system uid entries do not produce app sharing notice`() {
        assertEquals(emptySet<String>(), packagesSharingAppIds(mapOf("first" to listOf(1_000), "second" to listOf(1_000))))
    }
}
