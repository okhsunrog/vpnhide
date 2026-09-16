package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.ActiveDiagnosticRun
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRequest
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunEffect
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunEvent
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunState
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.MeasurementContext
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.reduceDiagnosticRun
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DiagnosticRunDataTest {
    @Test
    fun `retry publishes new identity synchronously and cannot complete from old failure`() {
        val fixture = RunFixture()
        fixture.request()
        val old = fixture.active.ticket
        fixture.send(DiagnosticRunEvent.Failed(old, TransitionFailure.ReadFailed))
        fixture.request()
        assertEquals(2L, fixture.active.id)
        assertEquals(DiagnosticStage.Checking, fixture.active.stage)
        fixture.send(DiagnosticRunEvent.ContextReady(old, runContext()))
        assertEquals(DiagnosticStage.Checking, fixture.active.stage)
        assertEquals(1, fixture.completed.size)
        fixture.complete()
        assertEquals(listOf(1L, 2L), fixture.completed.map { it.id })
        assertEquals(RunOutcome.Completed, fixture.completed.last().outcome)
    }

    @Test
    fun `same request joins active run without replacing execution identity`() {
        val fixture = RunFixture()
        fixture.request()
        fixture.context()
        val before = fixture.active
        fixture.request()
        assertEquals(before, fixture.active)
        assertEquals(DiagnosticRunEffect.Accepted(1, joined = true), fixture.effects.last())
        assertEquals(1, fixture.effects.filterIsInstance<DiagnosticRunEffect.Probe>().size)
    }

    @Test
    fun `slow probe failure retains partial evidence and previous complete measurement`() {
        val fixture = RunFixture()
        fixture.request()
        fixture.complete()
        val previous = fixture.state.lastComplete
        fixture.request()
        fixture.context()
        fixture.core()
        fixture.send(DiagnosticRunEvent.Failed(fixture.active.ticket, TransitionFailure.ExecutionFailed))
        assertEquals(DiagnosticStage.Draining, fixture.active.stage)
        fixture.drain()
        assertEquals(previous, fixture.state.lastComplete)
        assertEquals(RunOutcome.Failed, fixture.state.lastAttempt?.outcome)
        assertEquals(
            CheckOutcome.HiddenByBackend,
            fixture.state.lastAttempt
                ?.measurement
                ?.outcomes
                ?.get("core"),
        )
    }

    @Test
    fun `mutation during probes interrupts context and ignores delayed response`() {
        val fixture = RunFixture()
        fixture.request()
        fixture.context()
        val obsolete = fixture.active.ticket
        fixture.send(DiagnosticRunEvent.ContextChanged())
        fixture.send(DiagnosticRunEvent.ProbesFinished(obsolete, mapOf("core" to CheckOutcome.HiddenByBackend)))
        assertEquals(DiagnosticStage.Draining, fixture.active.stage)
        assertTrue(fixture.active.outcomes.isEmpty())
        fixture.drain()
        assertEquals(RunOutcome.Interrupted, fixture.completed.single().outcome)
        assertTrue(requireNotNull(fixture.completed.single().measurement).interrupted)
        assertEquals(null, fixture.state.lastComplete)
    }

    @Test
    fun `changed end context cannot certify completed probes`() {
        val fixture = RunFixture()
        fixture.request()
        fixture.context()
        fixture.core()
        fixture.slow()
        fixture.context(runContext().copy(changeEpoch = 2))
        assertEquals(RunOutcome.Interrupted, fixture.completed.single().outcome)
        assertEquals(TransitionFailure.ContextChanged, fixture.completed.single().failure)
        assertEquals(
            2L,
            fixture.completed
                .single()
                .measurement
                ?.endContext
                ?.changeEpoch,
        )
    }

    @Test
    fun `failed end observation is explicit uncertainty rather than green verdict`() {
        val fixture = RunFixture()
        fixture.request()
        fixture.context()
        fixture.core()
        fixture.slow()
        fixture.send(DiagnosticRunEvent.Failed(fixture.active.ticket, TransitionFailure.ReadFailed))
        assertEquals(TransitionFailure.ContextUnknown, fixture.completed.single().failure)
        assertEquals(null, fixture.state.lastComplete)
    }

    @Test
    fun `unproven drain resolves waiters and quarantines successor probes`() {
        val fixture = RunFixture()
        fixture.request()
        fixture.context()
        fixture.request(captureId = 42)
        fixture.send(DiagnosticRunEvent.Cancel(1))
        val staleDrain = fixture.active.ticket
        fixture.drain(quiescent = false)
        assertTrue(fixture.state.quarantined)
        assertEquals(listOf(1L, 2L), fixture.completed.map { it.id })
        assertEquals(TransitionFailure.ResourceUnavailable, fixture.completed.last().failure)
        fixture.request()
        assertEquals(null, fixture.state.active)
        fixture.send(DiagnosticRunEvent.Drained(staleDrain, true))
        assertTrue(fixture.state.quarantined)
        fixture.send(DiagnosticRunEvent.ResourceRecovered(staleDrain))
        fixture.request()
        assertEquals(DiagnosticStage.Checking, fixture.active.stage)
    }

    @Test
    fun `second deadline while draining terminates without falsely acknowledging helper cleanup`() {
        val fixture = RunFixture()
        fixture.request()
        fixture.context()
        fixture.send(DiagnosticRunEvent.Expired(1))
        assertEquals(DiagnosticStage.Draining, fixture.active.stage)
        fixture.send(DiagnosticRunEvent.Expired(1))
        assertFalse(fixture.state.quarantined)
        fixture.send(DiagnosticRunEvent.DrainExpired(fixture.active.ticket))
        assertTrue(fixture.state.quarantined)
        assertEquals(1, fixture.completed.size)
        fixture.send(DiagnosticRunEvent.Expired(1))
        assertEquals(1, fixture.completed.size)
    }

    @Test
    fun `waiting on a paused save resolves request without launching a probe`() {
        val fixture = RunFixture()
        fixture.request(dependencies = setOf(9))
        assertEquals(DiagnosticStage.Waiting, fixture.active.stage)
        fixture.send(DiagnosticRunEvent.OperationSettled(9, TransitionFailure.ApplicationUnknown))
        assertEquals(TransitionFailure.ApplicationUnknown, fixture.completed.single().failure)
        assertTrue(fixture.effects.none { it is DiagnosticRunEffect.Probe || it is DiagnosticRunEffect.Observe })
    }

    @Test
    fun `settled dependencies initiate a fresh observation before any probes`() {
        val fixture = RunFixture()
        fixture.request(dependencies = setOf(3, 4))
        fixture.send(DiagnosticRunEvent.OperationSettled(3))
        assertEquals(DiagnosticStage.Waiting, fixture.active.stage)
        fixture.send(DiagnosticRunEvent.OperationSettled(4))
        assertEquals(DiagnosticStage.Checking, fixture.active.stage)
        assertEquals(1, fixture.effects.filterIsInstance<DiagnosticRunEffect.Observe>().size)
    }

    @Test
    fun `different capture identity waits instead of joining an active suite`() {
        val fixture = RunFixture()
        fixture.request()
        fixture.context()
        fixture.request(captureId = 42)
        assertEquals(2L, fixture.state.pending?.id)
        fixture.request(captureId = 99)
        assertTrue(fixture.effects.last() is DiagnosticRunEffect.Rejected)
        fixture.core()
        fixture.slow()
        fixture.context()
        assertEquals(2L, fixture.active.id)
        assertEquals(42L, fixture.active.request.captureId)
        assertEquals(DiagnosticStage.Checking, fixture.active.stage)
    }

    @Test
    fun `automatic intent is consumed on probe start but not on blocked attempt`() {
        val fixture = RunFixture()
        fixture.request(automatic = true)
        fixture.send(DiagnosticRunEvent.Failed(fixture.active.ticket, TransitionFailure.ReadFailed))
        assertTrue(fixture.state.automaticAvailable)
        fixture.request(automatic = true)
        fixture.complete()
        assertFalse(fixture.state.automaticAvailable)
        val before = fixture.state
        fixture.request(automatic = true)
        assertEquals(before, fixture.state)
        fixture.request()
        assertEquals(3L, fixture.active.id)
    }

    @Test
    fun `obsolete probe completion cannot affect successor run after cancellation`() {
        val fixture = RunFixture()
        fixture.request()
        fixture.context()
        val obsolete = fixture.active.ticket
        fixture.request(captureId = 7)
        fixture.send(DiagnosticRunEvent.Cancel(1))
        fixture.drain()
        val successor = fixture.active
        fixture.send(DiagnosticRunEvent.ProbesFinished(obsolete, mapOf("core" to CheckOutcome.Leak)))
        assertEquals(successor, fixture.active)
        assertEquals(1, fixture.completed.size)
    }

    @Test
    fun `new accepted mutation delays admission and retires old context response`() {
        val fixture = RunFixture()
        fixture.request()
        val obsolete = fixture.active.ticket
        fixture.send(DiagnosticRunEvent.OperationAccepted(5))
        fixture.context()
        assertEquals(DiagnosticStage.Waiting, fixture.active.stage)
        fixture.send(DiagnosticRunEvent.OperationSettled(5))
        val current = fixture.active.ticket
        fixture.send(DiagnosticRunEvent.ContextReady(obsolete, runContext()))
        assertEquals(current, fixture.active.ticket)
        assertEquals(DiagnosticStage.Checking, fixture.active.stage)
        fixture.context()
        assertEquals(DiagnosticStage.Core, fixture.active.stage)
    }

    @Test
    fun `blocked checking observation finishes with its reason and keeps the automatic intent`() {
        val fixture = RunFixture()
        fixture.request(automatic = true)
        fixture.send(DiagnosticRunEvent.NotEligible(fixture.active.ticket, DiagnosticEligibility.VpnOff))
        val attempt = fixture.completed.single()
        assertEquals(RunOutcome.NotStarted, attempt.outcome)
        assertEquals(DiagnosticEligibility.VpnOff, attempt.eligibility)
        assertEquals(null, attempt.failure)
        assertEquals(null, attempt.measurement)
        assertTrue(fixture.state.automaticAvailable)
        assertTrue(fixture.effects.none { it is DiagnosticRunEffect.Probe })
        // Only a Checking observation can block; a late one after probes started is not a new outcome.
        fixture.request(automatic = true)
        fixture.context()
        fixture.send(DiagnosticRunEvent.NotEligible(fixture.active.ticket, DiagnosticEligibility.VpnOff))
        assertEquals(DiagnosticStage.Core, fixture.active.stage)
    }

    @Test
    fun `a request with different dependencies joins the active run instead of queueing a second suite`() {
        val fixture = RunFixture()
        fixture.request(dependencies = setOf(4))
        fixture.request()
        assertEquals(DiagnosticRunEffect.Accepted(1, joined = true), fixture.effects.last())
        assertEquals(null, fixture.state.pending)
        fixture.send(DiagnosticRunEvent.OperationSettled(4))
        fixture.request(dependencies = setOf(9))
        assertEquals(DiagnosticRunEffect.Accepted(1, joined = true), fixture.effects.last())
        assertEquals(DiagnosticStage.Checking, fixture.active.stage)
    }
}

internal fun runContext(): MeasurementContext = MeasurementContext("process:1/uid:10", "self-config:1", "vpn:1", "kpm:1", 1, 1, 10)

private class RunFixture {
    var state = DiagnosticRunState()
    val effects = mutableListOf<DiagnosticRunEffect>()
    private var now = 10L
    val active: ActiveDiagnosticRun get() = requireNotNull(state.active)
    val completed get() = effects.filterIsInstance<DiagnosticRunEffect.Completed>().map { it.attempt }

    fun send(event: DiagnosticRunEvent) {
        val next = reduceDiagnosticRun(state, event, now++)
        state = next.state
        effects += next.effects
    }

    fun request(
        captureId: Long? = null,
        dependencies: Set<Long> = emptySet(),
        automatic: Boolean = false,
    ) = send(
        DiagnosticRunEvent.Request(
            DiagnosticRequest(listOf(ProbePlanEntry("core"), ProbePlanEntry("slow")), captureId, dependencies, automatic),
        ),
    )

    fun context(context: MeasurementContext = runContext()) = send(DiagnosticRunEvent.ContextReady(active.ticket, context))

    fun core() = send(DiagnosticRunEvent.ProbesFinished(active.ticket, mapOf("core" to CheckOutcome.HiddenByBackend)))

    fun slow() = send(DiagnosticRunEvent.ProbesFinished(active.ticket, mapOf("slow" to CheckOutcome.HiddenBySelinux)))

    fun drain(quiescent: Boolean = true) = send(DiagnosticRunEvent.Drained(active.ticket, quiescent))

    fun complete() {
        context()
        core()
        slow()
        context()
    }
}
