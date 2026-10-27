package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreNativeBackend
import uniffi.vpnhide_core_ffi.CoreNativeBootOutcome
import uniffi.vpnhide_core_ffi.CoreNativeBootStatus
import uniffi.vpnhide_core_ffi.CoreSelectionMode
import uniffi.vpnhide_core_ffi.CoreZygiskTargets

class CoreNativeBindingTest {
    @Test
    fun `typed native boot plan preserves selected identity and unknown outcomes`() {
        val record =
            CoreNativeBootStatus(
                "boot",
                4u,
                10u,
                20u,
                CoreNativeBootOutcome.Selected(CoreNativeBackend.KPM, CoreSelectionMode.ADOPTED, true),
                listOf(CoreNativeBackend.KPM),
                listOf(CoreNativeBackend.KPM),
                emptyList(),
                null,
                CoreZygiskTargets.NotInstalled,
                null,
                listOf("obsolete"),
            )
        val selected = record.toBootStatus()
        assertEquals(NativeBootOutcome.Selected(NativeBackendId.Kpm, NativeSelectionMode.Adopted, true), selected.outcome)
        assertEquals(listOf("obsolete"), selected.obsoleteModules)
        record.outcome = CoreNativeBootOutcome.Unknown("no live status")
        assertEquals(NativeBootOutcome.Unknown("no live status"), record.toBootStatus().outcome)
    }
}
