package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.ConfigReadiness
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticImpactEffect
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticImpactEvent
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticImpactState
import dev.okhsunrog.vpnhide.diagnostics.configReadiness
import dev.okhsunrog.vpnhide.diagnostics.operationAffectsSelfMeasurement
import dev.okhsunrog.vpnhide.diagnostics.reduceDiagnosticImpact
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DiagnosticImpactDataTest {
    private val self = "dev.okhsunrog.vpnhide"

    @Test
    fun `only edits of this app, global features and whole replacements affect the self measurement`() {
        assertTrue(relevant(field("apps", self, "java")))
        assertTrue(relevant(field("apps", self)))
        assertTrue(relevant(field("apps")))
        assertTrue(relevant(field("settings")))
        assertTrue(relevant(field("settings", "optionalFeatures")))
        assertTrue(relevant(spec(emptySet(), removesCanonical = true)))
        assertFalse(relevant(field("apps", "other.app", "native")))
        assertFalse(relevant(field("apps", "other.app")))
        assertFalse(relevant(field("debug")))
        assertFalse(relevant(field("debugSwitch")))
        assertFalse(relevant(field("settings", "autoHiddenPackages")))
        assertFalse(relevant(field("settings", "rememberSuperkey")))
        // The startup runtime reconcile: forced activation, no write.
        assertFalse(relevant(spec(emptySet())))
    }

    @Test
    fun `a relevant operation delays runs, its first dispatch interrupts them once, settling releases them`() {
        var state = DiagnosticImpactState()
        val accepted = reduceDiagnosticImpact(state, DiagnosticImpactEvent.Accepted(5, relevant = true))
        assertEquals(listOf(DiagnosticImpactEffect.DelayRuns(5)), accepted.effects)
        state = accepted.state
        assertEquals(ConfigReadiness.Applying, configReadiness(state))

        val persist = reduceDiagnosticImpact(state, DiagnosticImpactEvent.Dispatched(5, ConfigPhase.Persist))
        assertEquals(listOf(DiagnosticImpactEffect.InterruptRuns), persist.effects)
        assertEquals(1L, persist.state.changeEpoch)
        val native = reduceDiagnosticImpact(persist.state, DiagnosticImpactEvent.Dispatched(5, ConfigPhase.Native))
        assertTrue(native.effects.isEmpty())
        assertEquals(1L, native.state.changeEpoch)
        state = native.state

        val settled = reduceDiagnosticImpact(state, DiagnosticImpactEvent.Settled(5, null))
        assertEquals(listOf(DiagnosticImpactEffect.SettleRuns(5, null)), settled.effects)
        assertEquals(DiagnosticImpactState(changeEpoch = 1), settled.state)
        assertEquals(ConfigReadiness.Settled, configReadiness(settled.state))
    }

    @Test
    fun `irrelevant operations produce no effects and no epoch change`() {
        var state = DiagnosticImpactState()
        for (
        event in
        listOf(
            DiagnosticImpactEvent.Accepted(7, relevant = false),
            DiagnosticImpactEvent.Dispatched(7, ConfigPhase.Native),
            DiagnosticImpactEvent.Settled(7, null),
        )
        ) {
            val transition = reduceDiagnosticImpact(state, event)
            assertTrue(transition.effects.isEmpty())
            state = transition.state
        }
        assertEquals(DiagnosticImpactState(), state)
    }

    @Test
    fun `unresolved and failed outcomes shape readiness until a later recovery or success`() {
        var state = reduceDiagnosticImpact(DiagnosticImpactState(), DiagnosticImpactEvent.Accepted(3, relevant = true)).state
        state = reduceDiagnosticImpact(state, DiagnosticImpactEvent.Settled(3, TransitionFailure.ApplicationUnknown)).state
        assertEquals(ConfigReadiness.Unknown, configReadiness(state))
        state = reduceDiagnosticImpact(state, DiagnosticImpactEvent.Recovered(3, TransitionFailure.ApplicationFailed)).state
        assertEquals(ConfigReadiness.Failed, configReadiness(state))
        state = reduceDiagnosticImpact(state, DiagnosticImpactEvent.Accepted(4, relevant = true)).state
        assertEquals(ConfigReadiness.Applying, configReadiness(state))
        state = reduceDiagnosticImpact(state, DiagnosticImpactEvent.Settled(4, null)).state
        assertEquals(ConfigReadiness.Settled, configReadiness(state))
    }

    private fun relevant(spec: ConfigOperationSpec) = operationAffectsSelfMeasurement(spec, self)

    private fun field(vararg segments: String) = spec(setOf(ConfigField(segments.toList())))

    private fun spec(
        writes: Set<ConfigField>,
        removesCanonical: Boolean = false,
    ) = ConfigOperationSpec(OperationSource.Ui, writes, removesCanonical = removesCanonical)
}
