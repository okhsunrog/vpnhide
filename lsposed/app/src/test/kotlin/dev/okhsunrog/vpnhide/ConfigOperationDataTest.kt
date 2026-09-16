package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ConfigOperationDataTest {
    private val javaRole = ConfigField(listOf("apps", "example.app", "java"))
    private val javaHook = ConfigField(listOf("apps", "example.app", "java", "hooks"))
    private val nativeRole = ConfigField(listOf("apps", "example.app", "native"))

    @Test
    fun `queued operation prepares only after previous activation and preserves unrelated edit`() {
        val fixture = OperationFixture()
        fixture.submit()
        val firstRead = fixture.ticket
        fixture.submit()
        assertEquals(1, fixture.effects.filterIsInstance<ConfigOperationEffect.Prepare>().size)
        fixture.prepare { it.copy(debugSwitch = true, debug = true) }
        fixture.confirm()
        assertTrue(fixture.state.confirmed.debugSwitch)
        assertEquals(ConfigPhase.Native, fixture.state.active?.phase)
        assertEquals(1, fixture.state.queue.size)
        fixture.confirm()
        assertEquals(OperationStage.Preparing, fixture.state.active?.stage)
        fixture.prepare { it.copy(settings = it.settings.copy(autoHideVpnName = true)) }
        fixture.confirm()
        fixture.send(ConfigOperationEvent.Prepared(firstRead, CanonicalConfig(), CanonicalConfig()))
        assertTrue(fixture.state.confirmed.debugSwitch)
        assertTrue(fixture.state.confirmed.settings.autoHideVpnName)
        fixture.confirm()
        assertEquals(listOf(1L, 2L), fixture.completions.map { it.id })
    }

    @Test
    fun `failed activation retains persisted switch and exposes unattempted ports`() {
        val fixture = OperationFixture()
        fixture.submit(phases = listOf(ConfigPhase.Persist, ConfigPhase.Native, ConfigPhase.Ports))
        fixture.prepare { it.copy(debug = true, debugSwitch = true) }
        fixture.confirm()
        fixture.finish(PhaseOutcome.FailedKnown)
        assertTrue(fixture.state.confirmed.debugSwitch)
        val result = fixture.completions.single()
        assertEquals(PhaseOutcome.Confirmed, result.phases[ConfigPhase.Persist])
        assertEquals(PhaseOutcome.FailedKnown, result.phases[ConfigPhase.Native])
        assertEquals(PhaseOutcome.NotAttempted, result.phases[ConfigPhase.Ports])
    }

    @Test
    fun `failed persistence does not publish candidate or dispatch activation`() {
        val fixture = OperationFixture()
        fixture.submit()
        fixture.prepare { it.copy(debugSwitch = true) }
        fixture.finish(PhaseOutcome.FailedKnown)
        assertFalse(fixture.state.confirmed.debugSwitch)
        assertEquals(listOf(ConfigPhase.Persist), fixture.executions.map { it.phase })
    }

    @Test
    fun `unknown mutation gets exactly one repeated readback then rejects queued work`() {
        val fixture = OperationFixture()
        fixture.submit()
        fixture.prepare { it.copy(debugSwitch = true) }
        val persistTicket = fixture.ticket
        fixture.submit()
        fixture.finish(PhaseOutcome.Unknown)
        val firstRecovery = fixture.ticket
        fixture.send(ConfigOperationEvent.RecoveryFinished(firstRecovery))
        val secondRecovery = fixture.ticket
        fixture.send(ConfigOperationEvent.RecoveryFinished(firstRecovery))
        assertEquals(secondRecovery, fixture.ticket)
        fixture.send(ConfigOperationEvent.RecoveryFinished(secondRecovery))
        assertEquals(OperationStage.Held, fixture.state.active?.stage)
        assertEquals(2, fixture.effects.filterIsInstance<ConfigOperationEffect.Reconcile>().size)
        assertEquals(listOf(TransitionFailure.ApplicationUnknown, TransitionFailure.MutationPaused), fixture.completions.map { it.failure })
        fixture.send(ConfigOperationEvent.PhaseFinished(persistTicket, PhaseOutcome.Confirmed))
        fixture.submit()
        assertEquals(1, fixture.executions.size)
        assertEquals(OperationStage.Held, fixture.state.active?.stage)
        assertEquals(2, fixture.completions.size)
    }

    @Test
    fun `manual recovery does not redeliver result or replay skipped activation`() {
        val fixture = pausedOperation()
        fixture.send(ConfigOperationEvent.Recheck)
        val readback = fixture.ticket
        fixture.send(ConfigOperationEvent.Recheck)
        assertEquals(readback, fixture.ticket)
        fixture.send(ConfigOperationEvent.RecoveryFinished(readback))
        assertEquals(OperationStage.Held, fixture.state.active?.stage)
        fixture.send(ConfigOperationEvent.Recheck)
        fixture.send(
            ConfigOperationEvent.RecoveryFinished(
                fixture.ticket,
                ConfigRecoveryEvidence(
                    CanonicalConfig(debugSwitch = true),
                    mapOf(ConfigPhase.Persist to PhaseOutcome.Confirmed, ConfigPhase.Native to PhaseOutcome.NotAttempted),
                ),
            ),
        )
        assertEquals(null, fixture.state.active)
        assertEquals(1, fixture.completions.size)
        assertEquals(1, fixture.effects.filterIsInstance<ConfigOperationEffect.Recovered>().size)
        assertEquals(listOf(ConfigPhase.Persist), fixture.executions.map { it.phase })
        assertTrue(fixture.state.confirmed.debugSwitch)
    }

    @Test
    fun `incomplete recovery evidence cannot reopen mutation lane`() {
        val fixture = OperationFixture()
        fixture.submit()
        fixture.prepare { it }
        fixture.finish(PhaseOutcome.Unknown)
        fixture.send(
            ConfigOperationEvent.RecoveryFinished(
                fixture.ticket,
                ConfigRecoveryEvidence(CanonicalConfig(), emptyMap()),
            ),
        )
        assertEquals(OperationStage.Reconciling, fixture.state.active?.stage)
        assertEquals(1, fixture.state.active?.recoveryAttempt)
        assertTrue(fixture.completions.isEmpty())
    }

    @Test
    fun `draft registration during preparation rejects overlapping bridge before dispatch`() {
        val fixture = OperationFixture()
        fixture.submit(OperationSource.Bridge, setOf(javaRole))
        fixture.send(ConfigOperationEvent.DraftChanged(7, setOf(javaHook)))
        fixture.prepare { it }
        assertTrue(fixture.executions.isEmpty())
        assertEquals(TransitionFailure.UiEditConflict, fixture.completions.single().failure)
        fixture.submit(OperationSource.Bridge, setOf(nativeRole))
        fixture.prepare { it }
        assertEquals(1, fixture.executions.size)
    }

    @Test
    fun `late UI edit returns conflict with truthful persisted outcome`() {
        val fixture = OperationFixture()
        fixture.submit(OperationSource.Bridge, setOf(javaRole), listOf(ConfigPhase.Persist))
        fixture.prepare { it.copy(debugSwitch = true) }
        fixture.send(ConfigOperationEvent.DraftChanged(7, setOf(javaHook)))
        fixture.confirm()
        val result = fixture.completions.single()
        assertEquals(TransitionFailure.UiEditConflict, result.failure)
        assertEquals(PhaseOutcome.Confirmed, result.phases[ConfigPhase.Persist])
        assertTrue(result.draftPending)
        assertTrue(fixture.state.drafts.containsKey(7))
    }

    @Test
    fun `cancel retires preparation but cannot cancel dispatched root command`() {
        val fixture = OperationFixture()
        fixture.submit()
        val obsolete = fixture.ticket
        fixture.send(ConfigOperationEvent.Cancel(1))
        fixture.submit()
        fixture.send(ConfigOperationEvent.Prepared(obsolete, CanonicalConfig(), CanonicalConfig(debug = true)))
        assertTrue(fixture.executions.isEmpty())
        fixture.prepare { it }
        fixture.send(ConfigOperationEvent.Cancel(2))
        assertEquals(OperationStage.Executing, fixture.state.active?.stage)
        assertEquals(1, fixture.effects.filterIsInstance<ConfigOperationEffect.CancelRejected>().size)
    }

    @Test
    fun `secret failure is independent of canonical persistence`() {
        val fixture = OperationFixture()
        fixture.submit(phases = listOf(ConfigPhase.Persist, ConfigPhase.Secret, ConfigPhase.Native))
        fixture.prepare { it.copy(settings = it.settings.copy(rememberSuperkey = true)) }
        fixture.confirm()
        fixture.finish(PhaseOutcome.FailedKnown)
        assertTrue(fixture.state.confirmed.settings.rememberSuperkey)
        assertEquals(PhaseOutcome.NotAttempted, fixture.completions.single().phases[ConfigPhase.Native])
        assertEquals(TransitionFailure.ExecutionFailed, fixture.completions.single().failure)
    }

    private fun pausedOperation(): OperationFixture {
        val fixture = OperationFixture()
        fixture.submit()
        fixture.prepare { it.copy(debugSwitch = true) }
        fixture.finish(PhaseOutcome.Unknown)
        repeat(2) { fixture.send(ConfigOperationEvent.RecoveryFinished(fixture.ticket)) }
        return fixture
    }

    @Test
    fun `candidate snapshots detach collections owned by a completed read effect`() {
        val hooks = mutableListOf("first", "second")
        val apps = mutableMapOf("example.app" to CanonicalApp(java = true, javaHooks = hooks))
        val features = mutableSetOf("feature")
        val candidate = CanonicalConfig(apps = apps, settings = CanonicalSettings(optionalFeatures = features))
        val fixture = OperationFixture()
        fixture.submit(phases = listOf(ConfigPhase.Persist))
        fixture.prepare { candidate }
        hooks.clear()
        apps.clear()
        features.clear()
        fixture.confirm()
        assertEquals(
            listOf("first", "second"),
            fixture.state.confirmed.apps["example.app"]
                ?.javaHooks,
        )
        assertEquals(setOf("feature"), fixture.state.confirmed.settings.optionalFeatures)
    }

    @Test
    fun `conflict registration detaches mutable field paths and rejection does not refresh root`() {
        val path = mutableListOf("apps", "example.app", "java")
        val fixture = OperationFixture()
        fixture.send(ConfigOperationEvent.DraftChanged(7, setOf(ConfigField(path))))
        path.clear()
        fixture.submit(OperationSource.Bridge, setOf(javaRole))
        fixture.prepare { it }
        assertEquals(TransitionFailure.UiEditConflict, fixture.completions.single().failure)
        assertTrue(fixture.effects.none { it is ConfigOperationEffect.RefreshObservations })
    }
}

private class OperationFixture {
    var state = ConfigOperationState(CanonicalConfig())
    val effects = mutableListOf<ConfigOperationEffect>()
    val ticket get() = requireNotNull(state.active).ticket
    val executions get() = effects.filterIsInstance<ConfigOperationEffect.Execute>()
    val completions get() = effects.filterIsInstance<ConfigOperationEffect.Completed>().map { it.result }

    fun send(event: ConfigOperationEvent) {
        val next = reduceConfigOperation(state, event)
        state = next.state
        effects += next.effects
    }

    fun submit(
        source: OperationSource = OperationSource.Ui,
        writes: Set<ConfigField> = emptySet(),
        phases: List<ConfigPhase> = listOf(ConfigPhase.Persist, ConfigPhase.Native),
    ) = send(ConfigOperationEvent.Submit(ConfigOperationSpec(source, writes, phases)))

    fun prepare(transform: (CanonicalConfig) -> CanonicalConfig) =
        send(ConfigOperationEvent.Prepared(ticket, state.confirmed, transform(state.confirmed)))

    fun finish(outcome: PhaseOutcome) = send(ConfigOperationEvent.PhaseFinished(ticket, outcome))

    fun confirm() = finish(PhaseOutcome.Confirmed)
}
