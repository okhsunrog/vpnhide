package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.CONFIRMATION_MIN_INTERVAL_MS
import dev.okhsunrog.vpnhide.diagnostics.CONFIRMATION_SETTLE_MS
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttempt
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticMeasurement
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.MeasurementContext
import dev.okhsunrog.vpnhide.diagnostics.MeasurementKey
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RoutingKnowledge
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.SelfRouting
import dev.okhsunrog.vpnhide.diagnostics.confirmMeasurements
import dev.okhsunrog.vpnhide.diagnostics.confirmationHoldOff
import dev.okhsunrog.vpnhide.diagnostics.key
import dev.okhsunrog.vpnhide.diagnostics.owedConfirmation
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.yield
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DiagnosticConfirmationDataTest {
    private val sessionA = context("framework:101:tun0")
    private val sessionB = context("framework:102:tun0")

    @Test
    fun `owed when eligible and nothing covers the current key`() {
        assertEquals(sessionA.key, owedConfirmation(presentation(current = sessionA), claimed = null))
    }

    @Test
    fun `not owed without a current context or while not eligible`() {
        assertNull(owedConfirmation(presentation(current = null), null))
        assertNull(owedConfirmation(presentation(current = sessionA, eligibility = DiagnosticEligibility.SelfExcluded), null))
        assertNull(owedConfirmation(presentation(current = sessionA, eligibility = DiagnosticEligibility.Checking), null))
    }

    @Test
    fun `not owed beside a run in flight or a quarantined probe`() {
        assertNull(owedConfirmation(presentation(current = sessionA, activeRunId = 3), null))
        assertNull(owedConfirmation(presentation(current = sessionA, probeUnavailable = true), null))
    }

    @Test
    fun `a measurement that still applies covers its key`() {
        val measured = measurement(sessionA, completed = true)
        assertNull(owedConfirmation(presentation(current = sessionA, measurement = measured), null))
        // The split-tunnel case: the same process re-enters a re-established tunnel.
        assertEquals(sessionB.key, owedConfirmation(presentation(current = sessionB, measurement = measured), null))
    }

    @Test
    fun `an attempt under the current key answers it whatever its outcome`() {
        val failed = DiagnosticAttempt(4, RunOutcome.Failed, TransitionFailure.ReadFailed, measurement(sessionA, completed = false))
        assertNull(owedConfirmation(presentation(current = sessionA, lastAttempt = failed), null))
        val interrupted = DiagnosticAttempt(4, RunOutcome.Interrupted, TransitionFailure.ContextChanged, measurement(sessionA, false))
        assertNull(owedConfirmation(presentation(current = sessionA, lastAttempt = interrupted), null))
        assertEquals(sessionB.key, owedConfirmation(presentation(current = sessionB, lastAttempt = interrupted), null))
    }

    @Test
    fun `a blocked attempt or one without a context does not cover the key`() {
        val blocked = DiagnosticAttempt(4, RunOutcome.NotStarted, eligibility = DiagnosticEligibility.VpnOff)
        assertEquals(sessionA.key, owedConfirmation(presentation(current = sessionA, lastAttempt = blocked), null))
        val contextless = DiagnosticAttempt(4, RunOutcome.Failed, TransitionFailure.ReadFailed)
        assertEquals(sessionA.key, owedConfirmation(presentation(current = sessionA, lastAttempt = contextless), null))
        // ...but the key this owner already asked for is never asked for again.
        assertNull(owedConfirmation(presentation(current = sessionA, lastAttempt = contextless), claimed = sessionA.key))
    }

    @Test
    fun `hold off is the settle window stretched to the minimum spacing`() {
        assertEquals(CONFIRMATION_SETTLE_MS, confirmationHoldOff(now = 10_000, lastRequestedAt = null))
        assertEquals(CONFIRMATION_SETTLE_MS, confirmationHoldOff(now = 10_000, lastRequestedAt = 10_000 - CONFIRMATION_MIN_INTERVAL_MS))
        assertEquals(CONFIRMATION_MIN_INTERVAL_MS - 500, confirmationHoldOff(now = 10_000, lastRequestedAt = 9_500))
    }

    @Test
    fun `owner requests once per key and again only for a new key`() =
        runBlocking {
            val presentations = MutableStateFlow(presentation(current = null))
            val requested = mutableListOf<MeasurementKey>()
            val waits = mutableListOf<Long>()
            var now = 0L
            val owner =
                launch {
                    confirmMeasurements(
                        presentations,
                        request = {
                            requested += it
                            true
                        },
                        clock = { now },
                        wait = {
                            waits += it
                            yield()
                        },
                    )
                }

            suspend fun settle() = repeat(SETTLE_TURNS) { yield() }

            settle()
            assertEquals(emptyList<MeasurementKey>(), requested)

            presentations.value = presentation(current = sessionA, pending = true)
            settle()
            assertEquals(listOf(sessionA.key), requested)
            assertEquals(listOf(CONFIRMATION_SETTLE_MS), waits)

            // The claimed key makes the presentation stop pending: nothing is asked again.
            val contextless = DiagnosticAttempt(1, RunOutcome.Failed, TransitionFailure.ReadFailed)
            presentations.value = presentation(current = sessionA, lastAttempt = contextless)
            settle()
            assertEquals(listOf(sessionA.key), requested)

            // A new session is a new pending key, requested after the spacing from the previous request.
            now = 2_000
            presentations.value = presentation(current = sessionB, lastAttempt = contextless, pending = true)
            settle()
            assertEquals(listOf(sessionA.key, sessionB.key), requested)
            assertEquals(CONFIRMATION_MIN_INTERVAL_MS - 2_000, waits.last())

            owner.cancelAndJoin()
        }

    @Test
    fun `a presentation change during the settle window restarts it`() =
        runBlocking {
            val presentations = MutableStateFlow(presentation(current = sessionA, pending = true))
            val requested = mutableListOf<MeasurementKey>()
            var release = false
            val owner =
                launch {
                    confirmMeasurements(
                        presentations,
                        request = {
                            requested += it
                            true
                        },
                        clock = { 0 },
                        wait = { while (!release) yield() },
                    )
                }
            repeat(SETTLE_TURNS) { yield() }
            // The tunnel settles into another identity before the first request went out:
            // the wait for the first key is cancelled, only the second key is requested.
            presentations.value = presentation(current = sessionB, pending = true)
            repeat(SETTLE_TURNS) { yield() }
            release = true
            repeat(SETTLE_TURNS) { yield() }
            assertEquals(listOf(sessionB.key), requested)
            owner.cancelAndJoin()
        }

    private fun context(routing: String) = MeasurementContext("pid:1;uid:10", "self", routing, "backend=builtin", 0, 1, 10)

    private fun measurement(
        context: MeasurementContext,
        completed: Boolean,
    ) = DiagnosticMeasurement(
        runId = 1,
        context = context,
        plan = listOf(ProbePlanEntry("ioctl_flags")),
        outcomes = mapOf("ioctl_flags" to CheckOutcome.HiddenByBackend),
        completed = completed,
        interrupted = !completed,
        endedAt = 20,
    )

    private fun presentation(
        current: MeasurementContext?,
        eligibility: DiagnosticEligibility = DiagnosticEligibility.Eligible,
        measurement: DiagnosticMeasurement? = null,
        lastAttempt: DiagnosticAttempt? = measurement?.let { DiagnosticAttempt(it.runId, RunOutcome.Completed, measurement = it) },
        activeRunId: Long? = null,
        probeUnavailable: Boolean = false,
        pending: Boolean = false,
    ) = DiagnosticPresentation(
        eligibility = eligibility,
        routing = RoutingKnowledge.Known(SelfRouting.Routed, 0),
        activeRunId = activeRunId,
        activeRunAutomatic = activeRunId?.let { true },
        activeStage = null,
        activeResults = null,
        lastAttempt = lastAttempt,
        measurement = measurement,
        measurementResults = null,
        applicability = if (measurement == null) MeasurementApplicability.Absent else MeasurementApplicability.MatchesLastObservation,
        evidence = null,
        currentSuccess = false,
        probeUnavailable = probeUnavailable,
        currentKey = current?.key,
        confirmationPending = pending,
    )

    private companion object {
        const val SETTLE_TURNS = 8
    }
}
