package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.AppRoleSelection
import dev.okhsunrog.vpnhide.picker.NATIVE_TARGET_UID_CAPACITY
import dev.okhsunrog.vpnhide.picker.NativeTargetCapacityUsage
import dev.okhsunrog.vpnhide.picker.nativeCapacityIncreaseViolation
import dev.okhsunrog.vpnhide.picker.nativeTargetCapacityUsage
import org.junit.Assert.assertEquals
import org.junit.Test

class NativeCapacityDataTest {
    private val self = "dev.okhsunrog.vpnhide"

    @Test
    fun `native capacity reserves one self slot before the picker reaches the limit`() {
        val selected =
            (0 until NATIVE_TARGET_UID_CAPACITY - 1).map { offset ->
                AppRoleSelection(
                    packageName = "com.example.$offset",
                    uids = listOf(10_000 + offset),
                    native = true,
                )
            }
        val full = nativeUsage(selected)
        val overflow =
            nativeUsage(
                selected +
                    AppRoleSelection(
                        packageName = "com.example.overflow",
                        uids = listOf(20_000),
                        native = true,
                    ),
            )

        assertEquals(NATIVE_TARGET_UID_CAPACITY, full.used)
        assertEquals(0, full.overflow)
        assertEquals(1, overflow.overflow)
        assertEquals(overflow, nativeCapacityIncreaseViolation(full, overflow))
    }

    @Test
    fun `multi-profile native app consumes one slot per resolved uid`() {
        val selected =
            (0 until NATIVE_TARGET_UID_CAPACITY - 2).map { offset ->
                AppRoleSelection(
                    packageName = "com.example.$offset",
                    uids = listOf(10_000 + offset),
                    native = true,
                )
            }
        val current = nativeUsage(selected)
        val candidate =
            nativeUsage(
                selected +
                    AppRoleSelection(
                        packageName = "com.example.profiled",
                        uids = listOf(30_000, 1_030_000),
                        native = true,
                    ),
            )

        assertEquals(NATIVE_TARGET_UID_CAPACITY - 1, current.used)
        assertEquals(NATIVE_TARGET_UID_CAPACITY + 1, candidate.used)
        assertEquals(candidate, nativeCapacityIncreaseViolation(current, candidate))
    }

    @Test
    fun `shared native uid is counted once`() {
        val usage =
            nativeUsage(
                listOf(
                    AppRoleSelection("com.shared.one", uids = listOf(12_345), native = true),
                    AppRoleSelection("com.shared.two", uids = listOf(12_345), native = true),
                ),
            )

        assertEquals(2, usage.used)
    }

    @Test
    fun `platform aid does not spend native capacity`() {
        val usage =
            nativeUsage(
                listOf(
                    AppRoleSelection("android.platform", uids = listOf(1_000), native = true),
                ),
            )

        assertEquals(1, usage.used)
    }

    @Test
    fun `native packages missing from picker remain in capacity calculation`() {
        val usage =
            nativeTargetCapacityUsage(
                selfPkg = self,
                selections = listOf(AppRoleSelection("com.visible")),
                existingNativePackages = setOf("com.visible", "com.preserved"),
                packageUids =
                    mapOf(
                        self to listOf(10_100),
                        "com.preserved" to listOf(12_000, 1_012_000),
                    ),
            )

        assertEquals(3, usage.used)
    }

    @Test
    fun `self always reserves exactly one slot`() {
        val usage =
            nativeTargetCapacityUsage(
                selfPkg = self,
                selections = emptyList(),
                existingNativePackages = emptySet(),
                packageUids = mapOf(self to listOf(10_100, 1_010_100)),
            )

        assertEquals(1, usage.used)
    }

    @Test
    fun `reducing an already overflowing selection remains allowed`() {
        val current = NativeTargetCapacityUsage(used = 162)
        val reduced = NativeTargetCapacityUsage(used = 161)

        assertEquals(null, nativeCapacityIncreaseViolation(current, reduced))
    }

    private fun nativeUsage(selections: Collection<AppRoleSelection>): NativeTargetCapacityUsage =
        nativeTargetCapacityUsage(
            selfPkg = self,
            selections = selections,
            existingNativePackages = emptySet(),
            packageUids = emptyMap(),
        )
}
