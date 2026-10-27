package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.core.CorePaths
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.vpnhide_core_ffi.CoreCommand
import uniffi.vpnhide_core_ffi.CoreException
import uniffi.vpnhide_core_ffi.StateReply
import uniffi.vpnhide_core_ffi.SubmitReply
import java.util.concurrent.atomic.AtomicLong
import uniffi.vpnhide_core_ffi.CoreSession as NativeCoreSession

/** Last-good data remains available for display, while [current] gates decisions. */
internal data class CoreSessionUpdate(
    val state: CoreStateView? = null,
    val current: Boolean = false,
    val protocolSkew: Boolean = false,
    val sequence: Long = 0,
    val connectionGeneration: Long = 0,
)

/** Commands and an authoritative publication stream; no app-side root phase executor. */
internal interface CoreSessionClient {
    val updates: StateFlow<CoreSessionUpdate>

    suspend fun read(): CoreReply

    suspend fun allocateOperationId(): Long

    suspend fun submit(
        op: Long,
        command: CoreCommand,
    ): CoreReply

    suspend fun operation(op: Long): CoreReply
}

/** One native handle, one long poll and one ordering owner for the app process. */
internal class FfiCoreSessionClient(
    socketPath: String,
    scope: CoroutineScope,
    private val mutableUpdates: MutableStateFlow<CoreSessionUpdate> = MutableStateFlow(CoreSessionUpdate()),
) : CoreSessionClient {
    val native = NativeCoreSession(socketPath)
    override val updates = mutableUpdates.asStateFlow()
    private val publicationLock = Any()
    private var order = CorePublicationData()
    private val requests = AtomicLong()

    init {
        scope.launch {
            while (true) {
                val cursor = updates.value.takeIf { it.current }?.state
                val reply =
                    call { request ->
                        val state =
                            if (cursor ==
                                null
                            ) {
                                native.readState()
                            } else {
                                native.awaitState(cursor.epoch.toULong(), cursor.revision.toULong())
                            }
                        publish(request, state)
                        updates.value
                            .takeIf { it.current }
                            ?.state
                            ?.let { CoreReply.Observed(it) } ?: CoreReply.Uncertain
                    }
                if (reply !is CoreReply.Observed) delay(500)
            }
        }
    }

    override suspend fun read(): CoreReply =
        call { request ->
            val state = native.readState()
            publish(request, state)
            updates.value
                .takeIf { it.current }
                ?.state
                ?.let { CoreReply.Observed(it) } ?: CoreReply.Uncertain
        }

    override suspend fun allocateOperationId(): Long = rootRequest { native.allocateOperationId().toLong() }

    override suspend fun operation(op: Long): CoreReply =
        call { request ->
            val reply = native.operation(op.toULong())
            reply.state?.let { publish(request, it) }
            CoreReply.Observed(updates.value.takeIf { it.current }?.state, reply.record?.toRecord())
        }

    override suspend fun submit(
        op: Long,
        command: CoreCommand,
    ): CoreReply =
        call { request ->
            when (val reply = native.submitCommand(op.toULong(), command)) {
                is SubmitReply.Accepted -> {
                    reply.state?.let { publish(request, it) }
                    CoreReply.Accepted(reply.op.toLong(), reply.record?.toRecord(), updates.value.takeIf { it.current }?.state)
                }

                is SubmitReply.Refused -> {
                    reply.reason.toRefusal()
                }
            }
        }

    private fun publish(
        request: Long,
        state: StateReply,
    ) {
        synchronized(publicationLock) {
            order = acceptCorePublication(order, state.connectionGeneration.toLong(), request, state.state.toState())
            mutableUpdates.value = order.update
        }
    }

    /** Every privileged surface uses the same generation-aware failure owner. */
    suspend fun <T> rootRequest(block: suspend () -> T): T {
        val request = requests.incrementAndGet()
        return try {
            block()
        } catch (e: CoreException) {
            failed(request, e)
            throw e
        }
    }

    private fun failed(
        request: Long,
        error: CoreException,
    ) {
        val generation =
            when (error) {
                is CoreException.Unavailable -> error.generation.toLong()
                is CoreException.Transport -> error.generation.toLong()
                is CoreException.ProtocolSkew -> error.generation.toLong()
                else -> null
            }
        if (generation != null) {
            synchronized(publicationLock) {
                order = failCorePublication(order, generation, request, error is CoreException.ProtocolSkew)
                mutableUpdates.value = order.update
            }
        }
    }

    private suspend fun call(block: suspend (Long) -> CoreReply): CoreReply {
        val request = requests.incrementAndGet()
        return try {
            block(request)
        } catch (e: CancellationException) {
            throw e
        } catch (e: CoreException) {
            failed(request, e)
            when (e) {
                is CoreException.Unavailable -> CoreReply.Unreachable
                is CoreException.ProtocolSkew -> CoreReply.ProtocolSkew
                is CoreException.Refused -> e.reason.toRefusal()
                is CoreException.InvalidInput -> CoreReply.Refused(uniffi.vpnhide_core_ffi.CoreSubmitRefusal.Rejected("invalid input"))
                is CoreException.Transport -> CoreReply.Uncertain
            }
        } catch (_: LinkageError) {
            CoreReply.Unreachable
        }
    }
}

/** Shared by config, root probes, logger, backend status and routing projections. */
internal object CoreSessions {
    @Volatile private var attached: FfiCoreSessionClient? = null
    private val mutableUpdates = MutableStateFlow(CoreSessionUpdate())
    val updates = mutableUpdates.asStateFlow()
    val client: CoreSessionClient? get() = attached

    @Synchronized
    fun attach(
        context: Context,
        scope: CoroutineScope = CanonicalConfigRepository.processScope,
    ): FfiCoreSessionClient? {
        attached?.let { return it }
        val session =
            try {
                FfiCoreSessionClient(CorePaths.socket(context.applicationContext), scope, mutableUpdates)
            } catch (
                _: LinkageError,
            ) {
                return null
            }
        return session.also { session ->
            attached = session
            scope.launch {
                session.updates.collect { update ->
                    corePublishedDebug(update)?.let { VpnHideLog.enabled = it }
                }
            }
        }
    }
}
