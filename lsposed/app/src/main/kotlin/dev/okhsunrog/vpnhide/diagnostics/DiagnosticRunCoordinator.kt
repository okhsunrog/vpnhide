package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.EffectTicket
import dev.okhsunrog.vpnhide.TransitionFailure
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch

/** Whole-run deadline, armed at admission: eligibility check, both probe phases and end-context verification. */
internal const val DIAGNOSTIC_RUN_DEADLINE_MS = 120_000L

/** Separate cleanup deadline after cancellation, failure or a context change; expiry quarantines the probe resource. */
internal const val DIAGNOSTIC_DRAIN_DEADLINE_MS = 30_000L

private const val FINISHED_RETENTION = 8

/** One immutable publication: the reducer core plus the raw check evidence the pure core does not model. */
internal data class DiagnosticRunView(
    val core: DiagnosticRunState = DiagnosticRunState(),
    /** Partial evidence of the active run, present once its core phase returned. */
    val activeResults: CheckResults? = null,
    /** Evidence retained for the latest attempt and the latest complete measurement, keyed by run id. */
    val attemptResults: Map<Long, CheckResults> = emptyMap(),
)

internal data class DiagnosticRunResult(
    val attempt: DiagnosticAttempt,
    val results: CheckResults?,
)

/** Resolves once, for its own run id only. Dropping it never touches the run. */
internal class DiagnosticRunHandle(
    val id: Long,
    private val result: CompletableDeferred<DiagnosticRunResult>,
) {
    suspend fun await(): DiagnosticRunResult = result.await()
}

internal sealed interface DiagnosticAdmission {
    data class Accepted(
        val handle: DiagnosticRunHandle,
        val joined: Boolean,
    ) : DiagnosticAdmission

    data class Rejected(
        val reason: TransitionFailure,
    ) : DiagnosticAdmission
}

/** Identified I/O effects of a run. Failures are exceptions; a blocked eligibility is a value. */
internal interface DiagnosticRunIo {
    suspend fun observe(
        ticket: EffectTicket,
        stage: DiagnosticStage,
    ): DiagnosticContextObservation

    suspend fun probe(
        ticket: EffectTicket,
        stage: DiagnosticStage,
        plan: List<ProbePlanEntry>,
    ): CheckResults
}

/**
 * Process-owned execution of [reduceDiagnosticRun]. The reducer runs under one
 * short lock; effects run on the supplied process scope and report back by
 * ticket, so a late or obsolete response cannot publish. Waiters own neither the
 * run nor its cancellation; only [cancel] requests execution cancellation.
 * Draining joins the run's outstanding effect jobs rather than cancelling them:
 * a blocking probe helper cannot be interrupted, only awaited or quarantined.
 */
internal class DiagnosticRunCoordinator(
    private val scope: CoroutineScope,
    private val io: DiagnosticRunIo,
    private val clock: () -> Long = System::currentTimeMillis,
    private val runDeadline: suspend () -> Unit = { delay(DIAGNOSTIC_RUN_DEADLINE_MS) },
    private val drainDeadline: suspend () -> Unit = { delay(DIAGNOSTIC_DRAIN_DEADLINE_MS) },
) {
    private val lock = Any()
    private val mutableView = MutableStateFlow(DiagnosticRunView())
    val view: StateFlow<DiagnosticRunView> = mutableView.asStateFlow()
    private val waiters = mutableMapOf<Long, MutableList<CompletableDeferred<DiagnosticRunResult>>>()
    private val finished = LinkedHashMap<Long, DiagnosticRunResult>()
    private val deadlines = mutableMapOf<Long, Job>()
    private val work = mutableMapOf<Long, MutableSet<Job>>()

    /** Admission is decided synchronously; the handle follows the actual (possibly shared) run id. */
    fun request(request: DiagnosticRequest): DiagnosticAdmission =
        synchronized(lock) {
            val effects = dispatch(DiagnosticRunEvent.Request(request))
            val accepted = effects.filterIsInstance<DiagnosticRunEffect.Accepted>().firstOrNull()
            val rejected = effects.filterIsInstance<DiagnosticRunEffect.Rejected>().firstOrNull()
            when {
                // The run just admitted is live, or finished within this same dispatch (blocked
                // eligibility), so its handle always exists.
                accepted != null -> DiagnosticAdmission.Accepted(checkNotNull(handle(accepted.id)), accepted.joined)

                else -> DiagnosticAdmission.Rejected(checkNotNull(rejected).reason)
            }
        }

    /**
     * Join the active run or read the latest attempt; only a suite that never ran
     * starts one, as the startup intent. Every later automatic run is owed by a
     * presentation whose measurable world nothing covers (`owedConfirmation`).
     */
    fun ensure(request: DiagnosticRequest): DiagnosticRunHandle? =
        synchronized(lock) {
            val state = view.value.core
            val current = state.active ?: state.pending
            when {
                current != null -> handle(current.id)
                state.lastAttempt != null -> handle(state.lastAttempt.id)
                else -> (request(request.copy(automatic = true)) as? DiagnosticAdmission.Accepted)?.handle
            }
        }

    fun cancel(id: Long) {
        synchronized(lock) { dispatch(DiagnosticRunEvent.Cancel(id)) }
    }

    /** A relevant config operation was accepted: waiting/checking runs depend on it until it settles. */
    fun operationAccepted(id: Long) {
        synchronized(lock) { dispatch(DiagnosticRunEvent.OperationAccepted(id)) }
    }

    fun operationSettled(
        id: Long,
        failure: TransitionFailure?,
    ) {
        synchronized(lock) { dispatch(DiagnosticRunEvent.OperationSettled(id, failure)) }
    }

    /** A known relevant change: an active run is interrupted (draining) and never certified against the new state. */
    fun contextChanged(known: Boolean = true) {
        synchronized(lock) { dispatch(DiagnosticRunEvent.ContextChanged(known)) }
    }

    /**
     * A handle for a finished run (its retained result) or a live one (a waiter
     * that its completion resolves). Null for an id that is neither: a result
     * evicted from retention has no handle, rather than a deferred nobody will
     * ever complete (I5: every handle has its own result).
     */
    private fun handle(id: Long): DiagnosticRunHandle? {
        finished[id]?.let { return DiagnosticRunHandle(id, CompletableDeferred(it)) }
        val core = mutableView.value.core
        if (core.active?.id != id && core.pending?.id != id) return null
        val deferred = CompletableDeferred<DiagnosticRunResult>().also { waiters.getOrPut(id) { mutableListOf() } += it }
        return DiagnosticRunHandle(id, deferred)
    }

    private fun dispatch(event: DiagnosticRunEvent): List<DiagnosticRunEffect> {
        val previous = mutableView.value
        val transition = reduceDiagnosticRun(previous.core, event, clock())
        val next = transition.state
        val activeId = previous.core.active?.id
        val retained = previous.attemptResults.toMutableMap()
        val completed = transition.effects.filterIsInstance<DiagnosticRunEffect.Completed>()
        if (activeId != null && previous.activeResults != null && completed.any { it.attempt.id == activeId }) {
            retained[activeId] = previous.activeResults
        }
        val keep = setOfNotNull(next.lastAttempt?.id, next.lastComplete?.runId)
        mutableView.value =
            DiagnosticRunView(
                core = next,
                activeResults = previous.activeResults?.takeIf { next.active != null && next.active.id == activeId },
                attemptResults = retained.filterKeys { it in keep },
            )
        transition.effects.forEach(::effect)
        return transition.effects
    }

    private fun effect(effect: DiagnosticRunEffect) {
        when (effect) {
            is DiagnosticRunEffect.ArmDeadline -> {
                deadlines[effect.id] =
                    scope.launch {
                        runDeadline()
                        synchronized(lock) { dispatch(DiagnosticRunEvent.Expired(effect.id)) }
                    }
            }

            is DiagnosticRunEffect.ArmDrainDeadline -> {
                scope.launch {
                    drainDeadline()
                    synchronized(lock) { dispatch(DiagnosticRunEvent.DrainExpired(effect.ticket)) }
                }
            }

            is DiagnosticRunEffect.Observe -> {
                observe(effect.ticket)
            }

            is DiagnosticRunEffect.Probe -> {
                probe(effect)
            }

            is DiagnosticRunEffect.Drain -> {
                drain(effect.ticket)
            }

            is DiagnosticRunEffect.Completed -> {
                complete(effect.attempt)
            }

            is DiagnosticRunEffect.Accepted, is DiagnosticRunEffect.Rejected -> {
                // Returned to the requester by request(); nothing to execute.
            }
        }
    }

    private fun observe(ticket: EffectTicket) {
        val stage = requireNotNull(mutableView.value.core.active).stage
        track(ticket.owner) {
            val result = runCatching { io.observe(ticket, stage) }
            synchronized(lock) { observed(ticket, stage, result) }
        }
    }

    private fun observed(
        ticket: EffectTicket,
        stage: DiagnosticStage,
        result: Result<DiagnosticContextObservation>,
    ) {
        val observation = result.getOrNull()
        val event =
            when {
                observation == null -> {
                    DiagnosticRunEvent.Failed(ticket, TransitionFailure.ReadFailed)
                }

                stage == DiagnosticStage.Checking && observation.eligibility != DiagnosticEligibility.Eligible -> {
                    DiagnosticRunEvent.NotEligible(ticket, observation.eligibility)
                }

                observation.context == null -> {
                    DiagnosticRunEvent.Failed(ticket, TransitionFailure.ReadFailed)
                }

                else -> {
                    DiagnosticRunEvent.ContextReady(ticket, observation.context)
                }
            }
        dispatch(event)
    }

    private fun probe(effect: DiagnosticRunEffect.Probe) {
        track(effect.ticket.owner) {
            val result = runCatching { io.probe(effect.ticket, effect.stage, effect.plan) }
            synchronized(lock) { probed(effect, result) }
        }
    }

    private fun probed(
        effect: DiagnosticRunEffect.Probe,
        result: Result<CheckResults>,
    ) {
        val results = result.getOrNull()
        if (results == null) {
            dispatch(DiagnosticRunEvent.Failed(effect.ticket, TransitionFailure.ExecutionFailed))
            return
        }
        val current = mutableView.value
        val active = current.core.active
        if (active == null || active.ticket != effect.ticket) return
        // Raw evidence is retained next to the core; the reducer only sees planned, not yet recorded outcomes.
        mutableView.value = current.copy(activeResults = mergeDiagnosticEvidence(current.activeResults, effect.stage, results))
        val outcomes = plannedOutcomes(effect.plan, results.all).filterKeys { it !in active.outcomes }
        dispatch(DiagnosticRunEvent.ProbesFinished(effect.ticket, outcomes))
    }

    private fun drain(ticket: EffectTicket) {
        val jobs = work[ticket.owner].orEmpty().toList()
        scope.launch {
            jobs.joinAll()
            synchronized(lock) { dispatch(DiagnosticRunEvent.Drained(ticket, quiescent = true)) }
        }
    }

    private fun complete(attempt: DiagnosticAttempt) {
        deadlines.remove(attempt.id)?.cancel()
        val result = DiagnosticRunResult(attempt, mutableView.value.attemptResults[attempt.id])
        finished[attempt.id] = result
        while (finished.size > FINISHED_RETENTION) finished.remove(finished.keys.first())
        waiters.remove(attempt.id)?.forEach { it.complete(result) }
    }

    /** Effect jobs are registered before they start, so a drain or quarantine always sees them. */
    private fun track(
        owner: Long,
        body: suspend () -> Unit,
    ) {
        val job = scope.launch(start = CoroutineStart.LAZY) { body() }
        work.getOrPut(owner) { mutableSetOf() } += job
        job.invokeOnCompletion { synchronized(lock) { released(owner, job) } }
        job.start()
    }

    private fun released(
        owner: Long,
        job: Job,
    ) {
        val remaining = work[owner]?.also { it -= job }
        if (!remaining.isNullOrEmpty()) return
        work.remove(owner)
        val core = mutableView.value.core
        val quarantine = core.quarantineTicket
        if (core.quarantined && quarantine?.owner == owner) dispatch(DiagnosticRunEvent.ResourceRecovered(quarantine))
    }
}
