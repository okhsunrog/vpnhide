package dev.okhsunrog.vpnhide

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreNativeBackend
import uniffi.vpnhide_core_ffi.CoreNativeBootOutcome
import uniffi.vpnhide_core_ffi.CoreNativeBootStatus
import uniffi.vpnhide_core_ffi.CoreNativeSelectionDecision
import uniffi.vpnhide_core_ffi.CoreNativeSelectionEvidence
import uniffi.vpnhide_core_ffi.CoreSelectionMode
import uniffi.vpnhide_core_ffi.CoreZygiskTargets

class CoreNativeBindingTest {
    @Test
    fun `typed native boot plan preserves selected identity and unknown outcomes`() {
        val record =
            CoreNativeBootStatus(
                selectionEvidence(),
                null,
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
        assertEquals(
            true,
            selected.selectionEvidence
                ?.jsonObject
                ?.get("conflicting_hints")
                ?.jsonPrimitive
                ?.boolean,
        )
        assertEquals(
            false,
            selected.selectionEvidence
                ?.jsonObject
                ?.get("inventory_complete")
                ?.jsonPrimitive
                ?.boolean,
        )
        val exportJson = Json { encodeDefaults = true }
        assertEquals(
            selected.selectionEvidence,
            exportJson.encodeToJsonElement(NativeBootStatus.serializer(), selected).jsonObject["selectionEvidence"],
        )
        assertFalse(
            exportJson
                .encodeToJsonElement(
                    NativeBootStatus.serializer(),
                    selected.copy(selectionEvidence = null),
                ).jsonObject
                .containsKey("selectionEvidence"),
        )
        record.outcome = CoreNativeBootOutcome.Unknown("no live status")
        assertEquals(NativeBootOutcome.Unknown("no live status"), record.toBootStatus().outcome)
    }

    private fun selectionEvidence(): CoreNativeSelectionEvidence =
        CoreNativeSelectionEvidence(
            bootId = "boot",
            observedAtBootMs = 42u,
            kernelRelease = "6.1.157-android14",
            kernelMachine = "aarch64",
            coreAbi = "arm64",
            manifestSha256 = "a".repeat(64),
            manifestFileSha256 = "c".repeat(64),
            sourceCommit = "b".repeat(40),
            candidateKmis = listOf("android14-6.1"),
            unameHint = "android14-6.1",
            moduleHint = null,
            conflictingHints = true,
            modules = emptyList(),
            inventoryComplete = false,
            unboundFiles = 1u,
            decision = CoreNativeSelectionDecision.ReconcileResident(CoreNativeBackend.KPM),
        )
}
