package dev.okhsunrog.vpnhide.core

import android.content.Context
import android.util.Log
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import uniffi.vpnhide_core_ffi.CoreClientHandle
import uniffi.vpnhide_core_ffi.CoreFfiException
import uniffi.vpnhide_core_ffi.productionSocketPath
import uniffi.vpnhide_core_ffi.protocolVersion

/**
 * Dev/production FFI session onto the root core (step 7).
 *
 * - Reconnects by constructing a new handle after transport failure / daemon restart.
 * - On epoch change: takes a fresh snapshot and does **not** infer operation outcomes from
 *   state updates — callers use [getOperationJson] independently.
 * - Honors [CoreReplicaParser] freshness: last-good is display-only when Unknown.
 *
 * Feature flag: [CoreIpcPrefs.useCoreReplica]. Legacy [dev.okhsunrog.vpnhide.ConfigRootIo]
 * remains the default until this path is validated on device.
 */
class CoreFfiGateway(
    private val socketPath: String = productionSocketPath(),
) {
    private val replica = MutableStateFlow<CoreReplicaView?>(null)
    val replicaFlow: StateFlow<CoreReplicaView?> = replica.asStateFlow()

    @Volatile private var handle: CoreClientHandle? = null
    @Volatile private var epoch: Long = 0

    val lastEpoch: Long
        get() = epoch

    suspend fun ensureConnected(): CoreClientHandle {
        handle?.let { return it }
        return reconnect("initial")
    }

    suspend fun reconnect(reason: String): CoreClientHandle {
        Log.i(TAG, "connect socket=$socketPath reason=$reason protocol=${protocolVersion()}")
        val h = CoreClientHandle.connect(socketPath)
        val newEpoch = h.hello()
        val previous = epoch
        epoch = newEpoch
        handle = h
        val snap = CoreReplicaParser.parseSnapshot(h.currentStateJson())
        replica.value = snap
        if (previous != 0L && previous != newEpoch) {
            Log.i(TAG, "epoch changed $previous → $newEpoch; fresh snapshot config_rev=${snap.configRevision}")
        }
        return h
    }

    /**
     * Long-poll loop body: await next state; if hello epoch drifts (daemon restart under
     * a live handle), reconnect and publish a fresh snapshot.
     */
    suspend fun awaitNextParsed(): CoreReplicaView {
        val h = ensureConnected()
        return try {
            val json = h.awaitNextStateJson()
            val view = CoreReplicaParser.parseSnapshot(json)
            replica.value = view
            view
        } catch (e: CoreFfiException) {
            Log.w(TAG, "await_next transport: ${e.message}; reconnecting")
            val h2 = reconnect("await_next_fail")
            val view = CoreReplicaParser.parseSnapshot(h2.currentStateJson())
            replica.value = view
            view
        }
    }

    suspend fun submitEditJson(
        op: Long,
        baseRevision: Long,
        editJson: String,
    ): String {
        val h = ensureConnected()
        return try {
            h.submitEditJson(op.toULong(), baseRevision.toULong(), editJson)
        } catch (e: CoreFfiException) {
            val h2 = reconnect("submit_fail")
            h2.submitEditJson(op.toULong(), baseRevision.toULong(), editJson)
        }
    }

    /** Operation receipt query — independent of the state mirror. */
    suspend fun getOperationJson(op: Long): String {
        val h = ensureConnected()
        return try {
            h.getOperationJson(op.toULong())
        } catch (e: CoreFfiException) {
            val h2 = reconnect("get_op_fail")
            h2.getOperationJson(op.toULong())
        }
    }

    companion object {
        private const val TAG = "VpnHide-CoreFfi"

        fun production(): CoreFfiGateway = CoreFfiGateway(productionSocketPath())

        fun forSocket(path: String): CoreFfiGateway = CoreFfiGateway(path)

        /** True when the process should prefer core over legacy su orchestration. */
        fun enabled(context: Context): Boolean = CoreIpcPrefs.useCoreReplica(context)
    }
}
