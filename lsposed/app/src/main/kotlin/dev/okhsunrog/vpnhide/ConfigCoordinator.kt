package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.NativeTargetCapacityWarning
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

private sealed interface CoordinatorMessage {
    data class Submit(
        val mutation: CanonicalMutation,
        val completion: CompletableDeferred<CanonicalWriteResult>,
    ) : CoordinatorMessage

    data class Input(
        val event: ConfigOperationEvent,
    ) : CoordinatorMessage

    data class Initialized(
        val result: ConfigInitialization,
    ) : CoordinatorMessage

    data class Executed(
        val ticket: EffectTicket,
        val evidence: ConfigPhaseEvidence,
        val recovery: Boolean,
    ) : CoordinatorMessage

    data class Initialize(
        val completion: CompletableDeferred<ConfigCoordinatorMode>,
    ) : CoordinatorMessage

    data object Retry : CoordinatorMessage
}

/**
 * Lifecycle of accepted operations, published synchronously from the actor in
 * dispatch order: acceptance (before any effect), preparation (the write set as
 * the fresh config actually turned out, before the first dispatch), each mutating
 * root dispatch, and the one result delivery, plus a later manual recovery of an
 * unresolved one. Callbacks must only record or invalidate; they never read back
 * or wait.
 */
internal interface ConfigOperationObserver {
    fun accepted(
        id: Long,
        spec: ConfigOperationSpec,
    )

    /**
     * The operation's spec with the prepared write set merged in. A mutation that
     * carries only a `transform` declares nothing at submission, so this is the
     * first moment its real write set is known (transition contract §7).
     */
    fun prepared(
        id: Long,
        spec: ConfigOperationSpec,
    )

    fun dispatched(
        id: Long,
        phase: ConfigPhase,
    )

    fun settled(result: ConfigOperationResult)

    fun recovered(result: ConfigOperationResult)

    object None : ConfigOperationObserver {
        override fun accepted(
            id: Long,
            spec: ConfigOperationSpec,
        ) = Unit

        override fun prepared(
            id: Long,
            spec: ConfigOperationSpec,
        ) = Unit

        override fun dispatched(
            id: Long,
            phase: ConfigPhase,
        ) = Unit

        override fun settled(result: ConfigOperationResult) = Unit

        override fun recovered(result: ConfigOperationResult) = Unit
    }
}

/**
 * One actor publishes before launching identified effects. Its scope must belong to the process.
 * Cancelling a caller only stops that caller's wait; it never cancels an accepted root operation.
 */
internal class ConfigCoordinator(
    private val io: ConfigCoordinatorIo,
    private val scope: CoroutineScope,
    private val confirmed: (CanonicalConfig) -> Unit = {},
    refresh: suspend () -> Unit = {},
    private val manageLogging: Boolean = false,
    private val invalidateObservations: () -> Unit = {},
    private val observer: ConfigOperationObserver = ConfigOperationObserver.None,
) {
    private val messages = Channel<CoordinatorMessage>(Channel.UNLIMITED)
    private val refreshRequests = Channel<Unit>(Channel.CONFLATED)
    private val mutableView = MutableStateFlow(ConfigCoordinatorView())
    val view = mutableView.asStateFlow()
    private var core: ConfigOperationState? = null
    private var initializing = false
    private var initialized = false
    private var available = false
    private var logging = CaptureLoggingState(false)
    private val drafts = mutableMapOf<Long, Set<ConfigField>>()
    private val requests = linkedMapOf<Long, CoordinatorMessage.Submit>()
    private val capacities = mutableMapOf<Long, NativeTargetCapacityWarning>()
    private val initializationWaiters = mutableListOf<CompletableDeferred<ConfigCoordinatorMode>>()

    init {
        scope.launch { for (message in messages) consume(message) }
        scope.launch(Dispatchers.IO) { for (request in refreshRequests) runCatching { refresh() } }
    }

    suspend fun initialize(): ConfigCoordinatorMode {
        val completion = CompletableDeferred<ConfigCoordinatorMode>()
        messages.send(CoordinatorMessage.Initialize(completion))
        return completion.await()
    }

    suspend fun submit(mutation: CanonicalMutation): CanonicalWriteResult {
        val completion = CompletableDeferred<CanonicalWriteResult>()
        messages.send(CoordinatorMessage.Submit(mutation, completion))
        return completion.await()
    }

    fun draftChanged(
        id: Long,
        fields: Set<ConfigField>,
    ) {
        messages.trySend(CoordinatorMessage.Input(ConfigOperationEvent.DraftChanged(id, configFieldSnapshot(fields))))
    }

    fun retry() {
        messages.trySend(CoordinatorMessage.Retry)
    }

    private fun consume(message: CoordinatorMessage) {
        when (message) {
            is CoordinatorMessage.Initialize -> {
                if (initialized && !initializing) {
                    message.completion.complete(view.value.mode)
                } else {
                    initializationWaiters.add(message.completion)
                    if (!initializing) startInitialization(repeat = true)
                }
            }

            CoordinatorMessage.Retry -> {
                when {
                    initializing -> Unit
                    core?.active?.stage == OperationStage.Held -> input(ConfigOperationEvent.Recheck)
                    core == null -> startInitialization(repeat = false)
                }
            }

            is CoordinatorMessage.Initialized -> {
                initialized(message.result)
            }

            is CoordinatorMessage.Submit -> {
                submitRequest(message)
            }

            is CoordinatorMessage.Input -> {
                input(message.event)
            }

            is CoordinatorMessage.Executed -> {
                executed(message)
            }
        }
    }

    private fun startInitialization(repeat: Boolean) {
        initializing = true
        mutableView.value = view.value.copy(mode = ConfigCoordinatorMode.Initializing)
        scope.launch(Dispatchers.IO) {
            var result = runCatching { io.initialize() }.getOrDefault(ConfigInitialization(ConfigCoordinatorMode.Unavailable))
            if (repeat && result.mode in setOf(ConfigCoordinatorMode.Unavailable, ConfigCoordinatorMode.Paused)) {
                result = runCatching { io.initialize() }.getOrDefault(ConfigInitialization(ConfigCoordinatorMode.Unavailable))
            }
            messages.send(CoordinatorMessage.Initialized(result))
        }
    }

    private fun initialized(result: ConfigInitialization) {
        initializing = false
        initialized = true
        available = result.config != null
        if (result.mode in setOf(ConfigCoordinatorMode.Open, ConfigCoordinatorMode.Missing)) {
            core = ConfigOperationState(canonicalConfigSnapshot(result.config ?: CanonicalConfig()), drafts = drafts.toMap())
        }
        mutableView.value = view.value.copy(mode = result.mode, confirmed = result.config?.let(::canonicalConfigSnapshot))
        result.config?.let { runCatching { confirmed(it) } }
        initializationWaiters.forEach { it.complete(result.mode) }
        initializationWaiters.clear()
    }

    private fun submitRequest(request: CoordinatorMessage.Submit) {
        request.mutation.captureEvent?.let { logging = reduceCaptureLogging(logging, it) }
        mutableView.value = view.value.copy(activeCaptures = logging.tokens.size)
        val state = core
        if (request.mutation.removesCanonical && logging.tokens.isNotEmpty()) {
            request.completion.complete(CanonicalWriteResult(-1, "capture_active"))
            return
        }
        if (state == null || initializing || (!available && !request.mutation.bootstrap && !request.mutation.removesCanonical)) {
            request.completion.complete(CanonicalWriteResult(-1, view.value.mode.name))
            return
        }
        requests[state.nextId] = request
        val spec =
            ConfigOperationSpec(
                request.mutation.source,
                request.mutation.writes,
                configMutationPlan(request.mutation, changed = true, loggingChanged = manageLogging && !request.mutation.removesCanonical),
                request.mutation.protectsDrafts,
                request.mutation.removesCanonical,
            )
        input(ConfigOperationEvent.Submit(spec))
    }

    private fun input(event: ConfigOperationEvent) {
        if (event is ConfigOperationEvent.DraftChanged) {
            if (event.fields.isEmpty()) drafts.remove(event.draftId) else drafts[event.draftId] = event.fields
        }
        val state = core ?: return
        if (event is ConfigOperationEvent.Prepared && event.ticket == state.active?.ticket &&
            state.active.stage == OperationStage.Preparing
        ) {
            available = event.baseAvailable
        }
        val transition = reduceConfigOperation(state, event)
        core = transition.state
        if (event is ConfigOperationEvent.PhaseFinished && event.ticket == state.active?.ticket &&
            event.outcome == PhaseOutcome.Confirmed && state.active.phase == ConfigPhase.Persist
        ) {
            available = true
        }
        if (event is ConfigOperationEvent.PhaseFinished && event.ticket == state.active?.ticket &&
            state.active.request.spec.removesCanonical && state.active.phase == ConfigPhase.Cleanup &&
            event.outcome == PhaseOutcome.Confirmed
        ) {
            available = false
        }
        publish()
        // The reducer has merged the prepared write set into the spec; observers see
        // it before this reduction's first Execute effect reports a mutating dispatch.
        if (event is ConfigOperationEvent.Prepared) {
            preparedSpec(event.ticket.owner)?.let { spec -> runCatching { observer.prepared(event.ticket.owner, spec) } }
        }
        transition.effects.forEach(::effect)
    }

    /** Null when the preparation did not leave this operation active (a conflict or an empty plan finished it). */
    private fun preparedSpec(id: Long): ConfigOperationSpec? =
        core
            ?.active
            ?.request
            ?.takeIf { it.id == id }
            ?.spec

    private fun publish() {
        val state = core ?: return
        val ids = (listOfNotNull(state.active?.request?.id) + state.queue.map { it.id }).filter { it in requests }.toSet()
        mutableView.value =
            view.value.copy(
                mode =
                    if (state.active?.resultDelivered ==
                        true
                    ) {
                        ConfigCoordinatorMode.Paused
                    } else if (available) {
                        ConfigCoordinatorMode.Open
                    } else {
                        ConfigCoordinatorMode.Missing
                    },
                confirmed = state.confirmed.takeIf { available },
                pending = pendingConfigToggles(ids.mapNotNull { requests[it]?.mutation }),
                operations = ids,
                rechecking = state.active?.stage == OperationStage.Reconciling,
            )
        if (available) runCatching { confirmed(state.confirmed) }
    }

    private fun effect(effect: ConfigOperationEffect) {
        when (effect) {
            is ConfigOperationEffect.Prepare -> {
                prepare(effect)
            }

            is ConfigOperationEffect.Execute -> {
                // Observers learn about the known change before the root effect starts.
                runCatching { observer.dispatched(effect.ticket.owner, effect.phase) }
                execute(effect)
            }

            is ConfigOperationEffect.Reconcile -> {
                val phase = requireNotNull(core?.active?.phase)
                scope.launch(Dispatchers.IO) {
                    val evidence =
                        runCatching {
                            io.recover(effect.ticket.owner, phase)
                        }.getOrDefault(ConfigPhaseEvidence(PhaseOutcome.Unknown, RootCanonicalRead.Unavailable))
                    messages.send(CoordinatorMessage.Executed(effect.ticket, evidence, true))
                }
            }

            is ConfigOperationEffect.Completed -> {
                runCatching { observer.settled(effect.result) }
                complete(effect.result)
            }

            is ConfigOperationEffect.Recovered -> {
                runCatching { observer.recovered(effect.result) }
                mutableView.value =
                    view.value.copy(
                        lastResult = effect.result,
                        recoveredResult = effect.result,
                        lastNativeCapacity = capacities.remove(effect.result.id),
                    )
                publish()
                val config = view.value.confirmed
                if (manageLogging && config != null && config.debug != (config.debugSwitch || logging.tokens.isNotEmpty())) {
                    messages.trySend(
                        CoordinatorMessage.Submit(
                            CanonicalMutation(emptyList(), source = OperationSource.System),
                            CompletableDeferred(),
                        ),
                    )
                }
            }

            is ConfigOperationEffect.Rejected -> {
                requests.remove(requireNotNull(core).nextId)?.completion?.complete(CanonicalWriteResult(-1, effect.reason.name))
                publish()
            }

            ConfigOperationEffect.RefreshObservations -> {
                refreshRequests.trySend(Unit)
            }

            is ConfigOperationEffect.Accepted -> {
                // Admission itself is published by input(); observers only learn the identity and intent.
                acceptedSpec(effect.id)?.let { spec -> runCatching { observer.accepted(effect.id, spec) } }
            }

            else -> {
                // CancelRejected needs no effect work.
            }
        }
    }

    private fun acceptedSpec(id: Long): ConfigOperationSpec? {
        val state = core ?: return null
        return state.active
            ?.request
            ?.takeIf { it.id == id }
            ?.spec ?: state.queue.firstOrNull { it.id == id }?.spec
    }

    private fun prepare(effect: ConfigOperationEffect.Prepare) {
        val mutation = requests.getValue(effect.ticket.owner).mutation
        val captureEnabled = logging.tokens.isNotEmpty().takeIf { manageLogging }
        scope.launch(Dispatchers.IO) {
            val event =
                runCatching {
                    prepareConfigMutation(effect.ticket, io.read(), mutation, captureEnabled)
                }.getOrElse { ConfigOperationEvent.PreparationFailed(effect.ticket, TransitionFailure.ReadFailed) }
            messages.send(CoordinatorMessage.Input(event))
        }
    }

    private fun execute(effect: ConfigOperationEffect.Execute) {
        val mutation = requests.getValue(effect.ticket.owner).mutation
        val base = core?.confirmed.takeIf { available }
        scope.launch(Dispatchers.IO) {
            val command = runCatching { configPhaseCommand(effect.phase, effect.config, mutation) }.getOrNull()
            val evidence =
                runCatching {
                    if (command ==
                        null
                    ) {
                        ConfigPhaseEvidence(PhaseOutcome.FailedKnown, base?.let(RootCanonicalRead::Available) ?: RootCanonicalRead.Missing)
                    } else {
                        io.execute(effect.ticket, effect.phase, base, effect.config, command)
                    }
                }.getOrDefault(ConfigPhaseEvidence(PhaseOutcome.Unknown, RootCanonicalRead.Unavailable))
            messages.send(CoordinatorMessage.Executed(effect.ticket, evidence, false))
        }
    }

    private fun executed(message: CoordinatorMessage.Executed) {
        val active = core?.active ?: return
        if (active.ticket != message.ticket) return
        // Advance observation generations before publishing phase evidence or resolving callers.
        runCatching { invalidateObservations() }
        val evidence = configCompletionEvidence(active, message.evidence)
        if (active.request.spec.removesCanonical && active.phase == ConfigPhase.Cleanup &&
            evidence.canonical == RootCanonicalRead.Missing &&
            evidence.outcome != PhaseOutcome.Unknown
        ) {
            available = false
        }
        evidence.nativeCapacity?.let { capacities[active.request.id] = it }
        if (message.recovery) {
            val recovered = configRecoveryEvidence(active, evidence, available)
            if (recovered != null) available = message.evidence.canonical is RootCanonicalRead.Available
            input(ConfigOperationEvent.RecoveryFinished(message.ticket, recovered))
        } else {
            input(ConfigOperationEvent.PhaseFinished(message.ticket, evidence.outcome))
        }
    }

    private fun complete(result: ConfigOperationResult) {
        val capacity = capacities.remove(result.id)
        mutableView.value = view.value.copy(lastResult = result, lastNativeCapacity = capacity)
        val request = requests.remove(result.id)
        publish()
        request?.completion?.complete(CanonicalWriteResult(if (result.failure == null) 0 else -1, nativeCapacityOutput(capacity), result))
    }
}
