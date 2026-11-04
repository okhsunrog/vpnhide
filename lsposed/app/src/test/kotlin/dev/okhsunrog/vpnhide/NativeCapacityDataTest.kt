package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.TargetsSnapshot
import dev.okhsunrog.vpnhide.picker.nativePickerSaveDecision
import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreNativeBackend
import uniffi.vpnhide_core_ffi.CoreNativePickerDecision
import uniffi.vpnhide_core_ffi.previewNativePickerChange
import uniffi.vpnhide_core_ffi.previewNativePolicy

/** UID/mask domain cases moved to native-policy; these pin the generated FFI and tap rule. */
class NativeCapacityDataTest {
    @Test
    fun `preview follows selected KPM rather than an unexpected observed builtin and preserves missing rows`() {
        val capacity = previewNativePolicy(CanonicalConfig().toCore(), CoreNativeBackend.KPM, emptyMap()).targetCapacity.toInt()
        val config = CanonicalConfig(apps = mapOf("many" to CanonicalApp(native = NativeRole.All)))
        val snapshot =
            TargetsSnapshot(
                emptyMap(),
                config,
                nativeStatus = sampleNativeStatus(selected = NativeBackendId.Kpm, observed = NativeBackendId.Builtin),
                packageUids = mapOf("many" to (2_010_000 until 2_010_000 + capacity).toList()),
                nativeInventoryComplete = true,
                capacityBackends = listOf(CoreNativeBackend.KPM, CoreNativeBackend.KMOD),
            )
        val decision = nativePickerSaveDecision(emptyList(), snapshot, APP_PACKAGE_NAME) as CoreNativePickerDecision.Capacity
        assertEquals(capacity.toULong(), decision.usage.targetUids)
        org.junit.Assert.assertTrue(decision.usage.wireBytes > decision.usage.wireCapacity)
        assertEquals(
            CoreNativePickerDecision.Allowed,
            nativePickerSaveDecision(emptyList(), snapshot.copy(capacityBackends = listOf(CoreNativeBackend.BUILTIN)), APP_PACKAGE_NAME),
        )
    }

    @Test
    fun `pure local reduction decision allows narrowing with no inventory or selected backend`() {
        val before = CanonicalConfig(apps = mapOf("target" to CanonicalApp(native = NativeRole.All)))
        val after =
            before.copy(
                apps =
                    mapOf("target" to CanonicalApp(native = NativeRole(true, NativeHookOverrides(kernel = listOf("dev_ioctl"))))),
            )
        assertEquals(
            CoreNativePickerDecision.Allowed,
            previewNativePickerChange(before.toCore(), after.toCore(), emptyList(), emptyMap(), false),
        )
        assertEquals(
            CoreNativePickerDecision.InventoryUnavailable,
            previewNativePickerChange(after.toCore(), before.toCore(), emptyList(), emptyMap(), false),
        )
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
