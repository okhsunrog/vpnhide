package dev.okhsunrog.vpnhide

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

private data class ConfigIntentRequest(
    val id: Long,
    val mutation: CanonicalMutation,
    val completion: CompletableDeferred<CanonicalWriteResult>,
    var writes: Set<ConfigField> = mutation.writes,
    var conflicts: Set<ConfigField> = emptySet(),
)

private data class ConfigIntentOutcome(
    val phases: Map<ConfigPhase, PhaseOutcome> = emptyMap(),
    val failure: TransitionFailure? = null,
    val nativeCapacity: NativeCapacityRefusal? = null,
    /** Only a core config operation can be queried; uncertain fixed effects are never replayed. */
    val uncertainOperation: Long? = null,
)

private sealed interface ConfigIntentMessage {
    data class Initialize(
        val completion: CompletableDeferred<ConfigCoordinatorMode>,
    ) : ConfigIntentMessage

    data class Initialized(
        val reply: CoreReply,
    ) : ConfigIntentMessage

    data class Publication(
        val update: CoreSessionUpdate,
    ) : ConfigIntentMessage

    data class Submit(
        val request: ConfigIntentRequest,
    ) : ConfigIntentMessage

    data class Draft(
        val id: Long,
        val fields: Set<ConfigField>,
    ) : ConfigIntentMessage

    data class Prepared(
        val request: ConfigIntentRequest,
        val reply: CoreReply,
    ) : ConfigIntentMessage

    data class Finished(
        val request: ConfigIntentRequest,
        val outcome: ConfigIntentOutcome,
        val recovery: Boolean = false,
    ) : ConfigIntentMessage

    data object Retry : ConfigIntentMessage
}

/**
 * Owns app intents, drafts and completion waiters. Core owns phase execution, admission and
 * recovery. A queued intent becomes one typed core command; cancelling a waiter cannot cancel it.
 */
internal class ConfigCoordinator(
    private val prepareCore: () -> CoreSessionClient?,
    private val scope: CoroutineScope,
    private val confirmed: (CanonicalConfig) -> Unit = {},
    refresh: suspend () -> Unit = {},
    private val invalidateObservations: () -> Unit = {},
    private val observer: ConfigOperationObserver = ConfigOperationObserver.None,
    private val newOperationId: () -> Long = CoreOperationIds::next,
) {
    private val messages = Channel<ConfigIntentMessage>(Channel.UNLIMITED)
    private val refreshRequests = Channel<Unit>(Channel.CONFLATED)
    private val mutableView = MutableStateFlow(ConfigCoordinatorView())
    val view = mutableView.asStateFlow()
    private val queue = ArrayDeque<ConfigIntentRequest>()
    private val drafts = mutableMapOf<Long, Set<ConfigField>>()
    private val initializers = mutableListOf<CompletableDeferred<ConfigCoordinatorMode>>()
    private var client: CoreSessionClient? = null
    private var publication = CoreSessionUpdate()
    private var active: ConfigIntentRequest? = null
    private var held: Pair<ConfigIntentRequest, Long?>? = null
    private var initializing = false

    init {
        scope.launch { for (message in messages) consume(message) }
        scope.launch { for (request in refreshRequests) runCatching { refresh() } }
    }

    suspend fun initialize(): ConfigCoordinatorMode {
        val result = CompletableDeferred<ConfigCoordinatorMode>()
        messages.send(ConfigIntentMessage.Initialize(result))
        return result.await()
    }

    suspend fun submit(mutation: CanonicalMutation): CanonicalWriteResult {
        val result = CompletableDeferred<CanonicalWriteResult>()
        messages.send(ConfigIntentMessage.Submit(ConfigIntentRequest(newOperationId(), mutation, result)))
        return result.await()
    }

    fun draftChanged(
        id: Long,
        fields: Set<ConfigField>,
    ) {
        messages.trySend(ConfigIntentMessage.Draft(id, configFieldSnapshot(fields)))
    }

    fun retry() {
        messages.trySend(ConfigIntentMessage.Retry)
    }

    private fun consume(message: ConfigIntentMessage) {
        when (message) {
            is ConfigIntentMessage.Initialize -> initialize(message.completion)
            is ConfigIntentMessage.Initialized -> initialized(message.reply)
            is ConfigIntentMessage.Publication -> acceptPublication(client?.updates?.value ?: message.update)
            is ConfigIntentMessage.Submit -> acceptIntent(message.request)
            is ConfigIntentMessage.Draft -> draft(message)
            is ConfigIntentMessage.Prepared -> prepared(message)
            is ConfigIntentMessage.Finished -> finished(message)
            ConfigIntentMessage.Retry -> retryReadback()
        }
    }

    private fun initialize(completion: CompletableDeferred<ConfigCoordinatorMode>) {
        if (client != null && publication.current) {
            completion.complete(view.value.mode)
            return
        }
        initializers.add(completion)
        if (initializing) return
        val session =
            client ?: prepareCore()?.also { attached ->
                client = attached
                scope.launch { attached.updates.collect { messages.send(ConfigIntentMessage.Publication(it)) } }
            }
        if (session == null) {
            initialized(CoreReply.Unreachable)
            return
        }
        initializing = true
        publish()
        scope.launch { messages.send(ConfigIntentMessage.Initialized(session.read())) }
    }

    private fun initialized(reply: CoreReply) {
        initializing = false
        updateFromReply(reply)
        publish()
        initializers.forEach { it.complete(view.value.mode) }
        initializers.clear()
        startNext()
    }

    private fun acceptPublication(update: CoreSessionUpdate) {
        val before = publication.state
        publication = update
        acceptCoreFacts(before, update)
        publish()
        observeHeldOperation(update)
        startNext()
    }

    private fun acceptCoreFacts(
        before: CoreStateView?,
        update: CoreSessionUpdate,
    ) {
        val after = update.state
        if (update.current && after != null) {
            if (before?.canonical != after.canonical || before.nativeBoot != after.nativeBoot || before.backend != after.backend) {
                invalidateObservations()
                refreshRequests.trySend(Unit)
            }
            (after.canonical as? RootCanonicalRead.Available)?.config?.let { confirmed(it) }
        } else if (before != null && view.value.mode != ConfigCoordinatorMode.Unavailable) {
            invalidateObservations()
        }
    }

    private fun observeHeldOperation(update: CoreSessionUpdate) {
        val pending = held?.second ?: return
        val state = update.state ?: return
        if (update.current && pending !in state.operationsInFlight && !state.mutationHeld) retryReadback()
    }

    private fun acceptIntent(request: ConfigIntentRequest) {
        publish()
        val failure =
            when {
                request.mutation.removesCanonical && (publication.state?.activeCaptures ?: 0) > 0 -> "capture_active"

                (held != null || publication.state?.mutationHeld == true) &&
                    !recoveryAllowed(publication.state?.recovery, request.mutation) -> "MutationPaused"

                !publication.current || initializing -> view.value.mode.name

                publication.state?.canonical !is RootCanonicalRead.Available && !request.mutation.bootstrap &&
                    !request.mutation.removesCanonical &&
                    !recoveryAllowed(
                        publication.state?.recovery,
                        request.mutation,
                    )
                -> view.value.mode.name

                else -> null
            }
        if (failure != null) {
            request.completion.complete(CanonicalWriteResult(-1, failure))
            return
        }
        queue.addLast(request)
        observer.accepted(request.id, spec(request))
        publish()
        startNext()
    }

    private fun startNext() {
        if (active != null || queue.isEmpty() || !publication.current) return
        if (held != null && !recoveryAllowed(publication.state?.recovery, queue.first().mutation)) return
        val session = client ?: return
        val request = queue.removeFirst()
        active = request
        publish()
        scope.launch { messages.send(ConfigIntentMessage.Prepared(request, session.read())) }
    }

    private fun prepared(message: ConfigIntentMessage.Prepared) {
        if (active != message.request) return
        updateFromReply(message.reply)
        val state = publication.state.takeIf { publication.current && message.reply is CoreReply.Observed }
        val request = message.request
        val preparation =
            state?.let {
                prepareConfigIntent(it.canonical, request.mutation)
            }
        if (preparation !is ConfigIntentPreparation.Prepared) {
            val failure = (preparation as? ConfigIntentPreparation.Failed)?.reason ?: TransitionFailure.ReadFailed
            finished(ConfigIntentMessage.Finished(request, ConfigIntentOutcome(failure = failure)))
            return
        }
        request.writes = preparation.intent.writes
        request.conflicts = conflicts(request)
        observer.prepared(request.id, spec(request))
        if (request.conflicts.isNotEmpty()) {
            finished(ConfigIntentMessage.Finished(request, ConfigIntentOutcome(failure = TransitionFailure.UiEditConflict)))
            return
        }
        scope.launch {
            val outcome =
                runCatching { executeIntent(request, requireNotNull(state), preparation.intent) }
                    .getOrElse { ConfigIntentOutcome(failure = TransitionFailure.ValidationFailed) }
            messages.send(ConfigIntentMessage.Finished(request, outcome))
        }
    }

    private suspend fun executeIntent(
        request: ConfigIntentRequest,
        state: CoreStateView,
        intent: PreparedConfigIntent,
    ): ConfigIntentOutcome {
        val session = requireNotNull(client)
        val command = coreCommand(state.configRevision, request.mutation, intent) ?: return ConfigIntentOutcome()
        // The app action ID is local feedback identity. Only a core-issued ID can dispatch root work.
        val operation =
            runCatching { session.allocateOperationId() }.getOrElse {
                return ConfigIntentOutcome(failure = TransitionFailure.ReadFailed)
            }
        observer.dispatched(request.id, if (request.mutation.removesCanonical) ConfigPhase.Cleanup else ConfigPhase.Persist)
        val reply = runCatching { session.submit(operation, command) }.getOrDefault(CoreReply.Uncertain)
        return commandOutcome(operation, reply)
    }

    private suspend fun commandOutcome(
        op: Long,
        reply: CoreReply,
    ): ConfigIntentOutcome {
        if (reply ==
            CoreReply.Uncertain
        ) {
            return readbackOutcome(op, runCatching { requireNotNull(client).operation(op) }.getOrDefault(CoreReply.Uncertain))
        }
        val result = reply
        return when (result) {
            is CoreReply.Accepted -> {
                recordOutcome(op, result.record)
            }

            is CoreReply.Observed -> {
                recordOutcome(op, result.record)
            }

            is CoreReply.Refused -> {
                ConfigIntentOutcome(
                    failure =
                        if (result.reason is uniffi.vpnhide_core_ffi.CoreSubmitRefusal.Conflict) {
                            TransitionFailure.UiEditConflict
                        } else {
                            TransitionFailure.ExecutionFailed
                        },
                    nativeCapacity = result.nativeCapacity,
                )
            }

            CoreReply.Unreachable, CoreReply.ProtocolSkew -> {
                ConfigIntentOutcome(failure = TransitionFailure.ReadFailed)
            }

            CoreReply.Uncertain -> {
                ConfigIntentOutcome(failure = TransitionFailure.ApplicationUnknown, uncertainOperation = op)
            }
        }
    }

    /** Only an actual receipt can resolve an already uncertain dispatch. */
    private fun readbackOutcome(
        op: Long,
        reply: CoreReply,
    ): ConfigIntentOutcome =
        when (reply) {
            is CoreReply.Observed -> recordOutcome(op, reply.record)
            is CoreReply.Accepted -> recordOutcome(op, reply.record)
            else -> ConfigIntentOutcome(failure = TransitionFailure.ApplicationUnknown, uncertainOperation = op)
        }

    private fun recordOutcome(
        op: Long,
        record: CoreOperationRecord?,
    ): ConfigIntentOutcome =
        if (record == null || record.settled == null) {
            ConfigIntentOutcome(record?.phases.orEmpty(), TransitionFailure.ApplicationUnknown, uncertainOperation = op)
        } else {
            ConfigIntentOutcome(
                record.phases,
                coreRecordFailure(record),
                uncertainOperation =
                    op.takeIf {
                        record.settled ==
                            PhaseOutcome.Unknown
                    },
            )
        }

    private fun finished(message: ConfigIntentMessage.Finished) {
        val request = message.request
        if (message.recovery) {
            if (held?.first != request) return
            mutableView.value = view.value.copy(rechecking = false)
            if (message.outcome.failure == TransitionFailure.ApplicationUnknown) return
            held = null
        } else if (active != request) {
            return
        }
        request.conflicts = request.conflicts + conflicts(request)
        val result =
            ConfigOperationResult(
                request.id,
                message.outcome.phases,
                when {
                    message.outcome.failure == TransitionFailure.ApplicationUnknown -> TransitionFailure.ApplicationUnknown
                    request.conflicts.isNotEmpty() -> TransitionFailure.UiEditConflict
                    else -> message.outcome.failure
                },
                request.conflicts,
                conflicts(request).isNotEmpty(),
            )
        if (message.outcome.failure == TransitionFailure.ApplicationUnknown) {
            held = request to message.outcome.uncertainOperation
        }
        if (recoveryAllowed(publication.state?.recovery, request.mutation) && message.outcome.failure == null) held = null
        active = null
        invalidateObservations()
        refreshRequests.trySend(Unit)
        mutableView.value =
            view.value.copy(
                lastResult = result,
                lastNativeCapacity = message.outcome.nativeCapacity,
                recoveredResult = result.takeIf { message.recovery } ?: view.value.recoveredResult,
            )
        deliverIntentResult(request, message, result)
        if (held != null) pauseQueuedIntents()
        publish()
        if (message.outcome.uncertainOperation != null) observeHeldOperation(publication)
        startNext()
    }

    private fun deliverIntentResult(
        request: ConfigIntentRequest,
        message: ConfigIntentMessage.Finished,
        result: ConfigOperationResult,
    ) {
        if (message.recovery) {
            observer.recovered(result)
        } else {
            observer.settled(result)
            request.completion.complete(
                CanonicalWriteResult(
                    if (result.failure ==
                        null
                    ) {
                        0
                    } else {
                        -1
                    },
                    "",
                    result,
                    message.outcome.nativeCapacity,
                ),
            )
        }
    }

    private fun pauseQueuedIntents() {
        while (queue.isNotEmpty()) {
            val pending = queue.removeFirst()
            val paused = ConfigOperationResult(pending.id, emptyMap(), TransitionFailure.MutationPaused)
            observer.settled(paused)
            pending.completion.complete(CanonicalWriteResult(-1, "MutationPaused", paused))
        }
    }

    private fun retryReadback() {
        val session = client ?: return
        if (view.value.rechecking) return
        mutableView.value = view.value.copy(rechecking = true)
        val pending = held
        scope.launch {
            val reply = session.read()
            messages.send(ConfigIntentMessage.Initialized(reply))
            if (pending != null) {
                val outcome =
                    pending.second?.let { readbackOutcome(it, session.operation(it)) }
                        ?: ConfigIntentOutcome(failure = TransitionFailure.ApplicationUnknown)
                messages.send(ConfigIntentMessage.Finished(pending.first, outcome, recovery = true))
            } else {
                messages.send(ConfigIntentMessage.Publication(session.updates.value))
            }
        }
    }

    private fun draft(message: ConfigIntentMessage.Draft) {
        if (message.fields.isEmpty()) drafts.remove(message.id) else drafts[message.id] = message.fields
        active?.let { it.conflicts = it.conflicts + conflicts(it) }
    }

    private fun conflicts(request: ConfigIntentRequest): Set<ConfigField> =
        if (request.mutation.protectsDrafts || request.mutation.source == OperationSource.Bridge) {
            conflictingFields(request.writes, drafts.values.flatten().toSet())
        } else {
            emptySet()
        }

    private fun spec(request: ConfigIntentRequest): ConfigOperationSpec =
        ConfigOperationSpec(
            request.mutation.source,
            request.writes,
            protectsDrafts = request.mutation.protectsDrafts,
            removesCanonical = request.mutation.removesCanonical,
        )

    private fun updateFromReply(
        @Suppress("UNUSED_PARAMETER") reply: CoreReply,
    ) {
        client?.updates?.value?.let(::acceptPublication)
    }

    private fun publish() {
        val state = publication.state
        val mode =
            when {
                initializing -> ConfigCoordinatorMode.Initializing
                held != null || state?.mutationHeld == true -> ConfigCoordinatorMode.Paused
                !publication.current -> ConfigCoordinatorMode.Unavailable
                state?.canonical == RootCanonicalRead.Missing -> ConfigCoordinatorMode.Missing
                state?.canonical == RootCanonicalRead.Invalid -> ConfigCoordinatorMode.Invalid
                state?.canonical is RootCanonicalRead.Available -> ConfigCoordinatorMode.Open
                else -> ConfigCoordinatorMode.Unavailable
            }
        val requests = listOfNotNull(active) + queue
        mutableView.value =
            view.value.copy(
                mode = mode,
                confirmed = (state?.canonical as? RootCanonicalRead.Available)?.config,
                coreCurrent = publication.current,
                coreEpoch = state?.epoch,
                coreSequence = publication.sequence,
                operations = requests.mapTo(linkedSetOf()) { it.id },
                pending = pendingConfigToggles(requests.map { it.mutation }),
                activeCaptures = state?.activeCaptures ?: 0,
                desiredRevision = state?.configRevision,
                nativeObservationCurrent = publication.current && state?.backend?.current == true,
                coreOperations = state?.operationsInFlight.orEmpty().toSet(),
                applyEvidence = state?.applyEvidence.orEmpty(),
                heldResources = state?.heldResources.orEmpty(),
                recovery = state?.recovery.takeIf { publication.current },
                startupError = state?.startupError,
                canRepair = publication.current && state?.recovery?.let { it.repair == null } == true,
                rechecking = view.value.rechecking && (held != null || initializing),
            )
    }
}
