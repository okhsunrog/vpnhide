package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.toJson
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class AutonomousExportTest {
    @Test
    fun `retained Core evidence never masquerades as this capture's legacy measurement`() {
        val state = sampleBundleState()
        val json = Json.parseToJsonElement(state.toJson()).jsonObject
        for (legacy in listOf("gate", "report", "selfTestRunId", "diagnostics", "nativeVerdict", "javaVerdict")) {
            assertEquals(legacy, JsonNull, json[legacy])
        }
        val autonomous = json.getValue("autonomousDiagnostics").jsonObject
        assertEquals("retained_core_publication", autonomous.getValue("provenance").jsonPrimitive.content)
        assertEquals(
            "99",
            autonomous
                .getValue("run")
                .jsonObject
                .getValue("session")
                .jsonPrimitive.content,
        )
        assertEquals(
            "incomplete",
            autonomous
                .getValue("assessment")
                .jsonObject
                .getValue("phase")
                .jsonPrimitive.content,
        )
        assertFalse(state.autonomousDiagnostics!!.run!!.loadedGenerationsComplete)
    }

    @Test
    fun `canonical export uses its Core publication even while actor and raw metadata are older`() {
        val current = CanonicalConfig(debug = true)
        val core = CoreSessionUpdate(CoreStateView(9, 3, RootCanonicalRead.Available(current), emptyList()), current = true, sequence = 12)
        val oldConfig =
            ConfigCoordinatorView(
                confirmed = CanonicalConfig(debug = false),
                coreCurrent = true,
                coreEpoch = 8,
                desiredRevision = 2,
                coreSequence = 11,
            )
        val root = RootSnapshot(mapOf("canonical_config" to canonicalConfigJson(CanonicalConfig(debug = false))))
        val sources =
            dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticSources(
                core,
                ObservationState(lastGood = ObservedValue(root, ObservationRequest(1, 0, 0, ReadReason.Background), 0)),
                oldConfig,
            )
        val snapshot =
            dev.okhsunrog.vpnhide.diagnostics
                .autonomousDiagnosticSnapshot(sources, 1, 0)
        assertEquals(
            Json.parseToJsonElement(canonicalConfigJson(current)),
            dev.okhsunrog.vpnhide.debug
                .autonomousExportCanonical(snapshot),
        )
        val offline =
            dev.okhsunrog.vpnhide.diagnostics
                .autonomousDiagnosticSnapshot(sources.copy(core = core.copy(current = false)), 2, 0)
        assertEquals(
            null,
            dev.okhsunrog.vpnhide.debug
                .autonomousExportCanonical(offline),
        )
    }

    @Test
    fun `historical runtime fields retain installation facts but withdraw active claims`() {
        val observed = ModuleState.Installed(version = "test", active = true, runtimeCheckable = true)
        val retained =
            dev.okhsunrog.vpnhide.debug
                .unverifiedExportRuntime(observed) as ModuleState.Installed
        assertEquals("test", retained.version)
        assertFalse(retained.active)
        assertFalse(retained.runtimeCheckable)
    }
}
