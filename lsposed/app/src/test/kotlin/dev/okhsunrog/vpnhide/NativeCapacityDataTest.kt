package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.TargetsSnapshot
import dev.okhsunrog.vpnhide.picker.nativeCapacityIncreaseViolation
import dev.okhsunrog.vpnhide.picker.nativeCapacityUsage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreNativeBackend
import uniffi.vpnhide_core_ffi.CoreNativePolicyUsage
import uniffi.vpnhide_core_ffi.nativePolicyIsReduction
import uniffi.vpnhide_core_ffi.previewNativePolicy

/** UID/mask domain cases moved to native-policy; these pin the generated FFI and tap rule. */
class NativeCapacityDataTest {
    @Test
    fun `preview follows selected KPM rather than an unexpected observed builtin and preserves missing rows`() {
        val capacity = previewNativePolicy(CanonicalConfig().toCore(), CoreNativeBackend.KPM, emptyMap()).targetCapacity.toInt()
        val config = CanonicalConfig(apps = mapOf("many" to CanonicalApp(native = NativeRole.All)))
        val snapshot =
            TargetsSnapshot(
                false,
                false,
                false,
                emptyMap(),
                config,
                activeNativeBackendId = NativeBackendId.Builtin,
                packageUids = mapOf("many" to (2_010_000 until 2_010_000 + capacity).toList()),
                nativeInventoryComplete = true,
                selectedNativeBackendId = NativeBackendId.Kpm,
            )
        val usage = requireNotNull(nativeCapacityUsage(emptyList(), snapshot, APP_PACKAGE_NAME))
        assertEquals(capacity.toULong(), usage.targetUids)
        org.junit.Assert.assertTrue(usage.wireBytes > usage.wireCapacity)
        val builtin =
            requireNotNull(
                nativeCapacityUsage(emptyList(), snapshot.copy(selectedNativeBackendId = NativeBackendId.Builtin), APP_PACKAGE_NAME),
            )
        org.junit.Assert.assertTrue(builtin.wireCapacity > usage.wireCapacity)
    }

    @Test
    fun `pure local reduction decision allows narrowing with no inventory or selected backend`() {
        val before = CanonicalConfig(apps = mapOf("target" to CanonicalApp(native = NativeRole.All)))
        val after =
            before.copy(
                apps =
                    mapOf("target" to CanonicalApp(native = NativeRole(true, NativeHookOverrides(kernel = listOf("dev_ioctl"))))),
            )
        org.junit.Assert.assertTrue(nativePolicyIsReduction(before.toCore(), after.toCore(), null))
        org.junit.Assert.assertFalse(nativePolicyIsReduction(after.toCore(), before.toCore(), null))
    }

    @Test
    fun `next selection is refused and removals from an overflowing draft remain possible`() {
        val full = CoreNativePolicyUsage(4u, 4u, 80u, 100u)
        val overflow = full.copy(targetUids = 5u, wireBytes = 90u)
        assertEquals(overflow, nativeCapacityIncreaseViolation(full, overflow))
        assertNull(nativeCapacityIncreaseViolation(overflow, full))
        assertNull(nativeCapacityIncreaseViolation(overflow.copy(targetUids = 6u), overflow))
    }

    @Test
    fun `independent wire capacity also prevents an increase`() {
        val current = CoreNativePolicyUsage(3u, 4u, 99u, 100u)
        val candidate = current.copy(wireBytes = 101u)
        assertEquals(candidate, nativeCapacityIncreaseViolation(current, candidate))
        assertNull(nativeCapacityIncreaseViolation(candidate, current))
    }

    @Test
    fun `local FFI uses Core family hooks real self uid and deduplicated complete inventory`() {
        val config =
            CanonicalConfig(
                apps =
                    mapOf(
                        APP_PACKAGE_NAME to CanonicalApp(native = NativeRole.All),
                        "shared" to CanonicalApp(native = NativeRole.All),
                        "other" to CanonicalApp(native = NativeRole.All),
                    ),
            )
        val inventory = mapOf(APP_PACKAGE_NAME to listOf(10123u, 110123u), "shared" to listOf(10123u), "other" to listOf(12000u, 112000u))
        val usage = previewNativePolicy(config.toCore(), CoreNativeBackend.KPM, inventory)
        assertEquals(3uL, usage.targetUids)
        val empty = previewNativePolicy(config.toCore(), CoreNativeBackend.KPM, emptyMap())
        assertEquals(0uL, empty.targetUids)
        assertEquals(usage.targetCapacity, empty.targetCapacity)
    }
}
