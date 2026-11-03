package dev.okhsunrog.vpnhide.maintenance

import android.content.Context
import android.net.Uri
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.CoreReply
import dev.okhsunrog.vpnhide.CoreSessions
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.vpnhide_core_ffi.CoreCommand
import uniffi.vpnhide_core_ffi.CoreMaintenanceStage
import uniffi.vpnhide_core_ffi.coreReplacementEnvelopeLimit
import uniffi.vpnhide_core_ffi.coreReplacementTarget

/** Android document adapter only. Core authenticates and owns durable execution. */
internal object CoreMaintenance {
    private val mutableBusy = MutableStateFlow(false)
    val busy = mutableBusy.asStateFlow()
    private val mutableIssue = MutableStateFlow(false)
    val issue = mutableIssue.asStateFlow()
    private val pendingOperation = MutableStateFlow<Long?>(null)
    val pending = pendingOperation.asStateFlow()

    fun targetAvailable(context: Context): Boolean = target(context) != null

    private fun target(context: Context) =
        runCatching {
            val bytes =
                context.assets.open("maintenance/core-target.json").use { input ->
                    val limit = coreReplacementEnvelopeLimit().toInt()
                    val output = java.io.ByteArrayOutputStream()
                    val buffer = ByteArray(4096)
                    while (output.size() <= limit) {
                        val n = input.read(buffer, 0, minOf(buffer.size, limit + 1 - output.size()))
                        if (n < 0) break
                        output.write(buffer, 0, n)
                    }
                    output.toByteArray()
                }
            coreReplacementTarget(bytes)
        }.getOrNull()

    fun stage(
        context: Context,
        uri: Uri,
    ) {
        if (pendingOperation.value != null || !mutableBusy.compareAndSet(expect = false, update = true)) return
        val app = context.applicationContext
        mutableIssue.value = false
        CanonicalConfigRepository.processScope.launch {
            try {
                val target = requireNotNull(withContext(Dispatchers.IO) { target(app) })
                val session = requireNotNull(CoreSessions.client)
                val state = (session.read() as? CoreReply.Observed)?.state
                requireNotNull(state)
                val staged =
                    withContext(Dispatchers.IO) {
                        requireNotNull(app.contentResolver.openInputStream(uri)).use { input ->
                            stageMaintenanceDocument(
                                app.filesDir,
                                input,
                                target.request.expectedPackageSha256,
                                target.packageBytes.toLong(),
                            )
                        }
                    }
                val operation = session.allocateOperationId()
                pendingOperation.value = operation
                when (session.submit(operation, CoreCommand.StageCoreReplacement(state.configRevision.toULong(), target.request))) {
                    is CoreReply.Accepted -> {
                        pendingOperation.value = null
                        withContext(Dispatchers.IO) { staged.delete() }
                    }

                    CoreReply.Uncertain -> {
                        // Query the SAME operation. Never repeat an installer command
                        // merely because its acknowledgement was lost.
                        observePending()
                    }

                    else -> {
                        pendingOperation.value = null
                        mutableIssue.value = true
                        withContext(Dispatchers.IO) { staged.delete() }
                    }
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                mutableIssue.value = true
            } finally {
                mutableBusy.value = false
            }
        }
    }

    fun refresh() {
        CanonicalConfigRepository.processScope.launch {
            runCatching { observePending() }
        }
    }

    private suspend fun observePending() {
        val operation = pendingOperation.value ?: return
        val observed = CoreSessions.client?.operation(operation) as? CoreReply.Observed
        val status = observed?.state?.maintenance?.takeIf { it.operation == operation.toULong() }
        if (observed?.record != null || status != null) pendingOperation.value = null
        mutableIssue.value = status == null || status.stage == CoreMaintenanceStage.UNKNOWN
    }
}

internal fun CoreMaintenanceStage.isRunning(): Boolean =
    this in
        setOf(
            CoreMaintenanceStage.ACCEPTED,
            CoreMaintenanceStage.DISPATCHING,
            CoreMaintenanceStage.VERIFYING_STAGED,
            CoreMaintenanceStage.RETIRING_LEGACY,
        )
