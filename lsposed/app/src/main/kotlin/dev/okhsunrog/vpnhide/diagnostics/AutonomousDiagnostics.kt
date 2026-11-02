package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ConfigCoordinatorView
import dev.okhsunrog.vpnhide.CoreSessionUpdate
import dev.okhsunrog.vpnhide.ObservationState
import dev.okhsunrog.vpnhide.RootSnapshot
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.vpnhide_core_ffi.CoreDiagnosticException
import uniffi.vpnhide_core_ffi.CoreDiagnosticRefusal
import uniffi.vpnhide_core_ffi.CoreException
import java.util.concurrent.atomic.AtomicBoolean

internal sealed interface AutonomousRetryFailure {
    data object Unavailable : AutonomousRetryFailure

    data class Refused(
        val reason: CoreDiagnosticRefusal,
    ) : AutonomousRetryFailure
}

/** Request progress is UI state, never a competing diagnostic outcome. */
internal data class AutonomousRetryState(
    val pending: Boolean = false,
    val failure: AutonomousRetryFailure? = null,
    val sourceSequence: Long = 0,
)

/** Same facade in production and tests. Only explicit retry may call [request]. */
internal class AutonomousDiagnostics(
    private val scope: CoroutineScope,
    private val core: StateFlow<CoreSessionUpdate>,
    root: StateFlow<ObservationState<RootSnapshot>>,
    config: StateFlow<ConfigCoordinatorView>,
    private val request: suspend () -> Unit,
) {
    private val owner = AutonomousDiagnosticSnapshotOwner(scope, core, root, config)
    val snapshot = owner.snapshot
    private val admitted = AtomicBoolean(false)
    private val mutableRetry = MutableStateFlow(AutonomousRetryState())
    val retryState = mutableRetry.asStateFlow()

    suspend fun awaitSnapshot(root: RootSnapshot? = null): AutonomousDiagnosticSnapshot = owner.awaitSnapshot(root)

    fun retry() {
        if (!admitted.compareAndSet(false, true)) return
        val sequence = core.value.sequence
        mutableRetry.value = AutonomousRetryState(pending = true, sourceSequence = sequence)
        scope.launch {
            var failure: AutonomousRetryFailure? = null
            try {
                request()
            } catch (error: CancellationException) {
                throw error
            } catch (error: CoreDiagnosticException.Refused) {
                failure = AutonomousRetryFailure.Refused(error.reason)
            } catch (_: CoreException) {
                // FfiCoreSessionClient already withdrew this generation's publication.
                failure = AutonomousRetryFailure.Unavailable
            } catch (_: LinkageError) {
                failure = AutonomousRetryFailure.Unavailable
            } finally {
                mutableRetry.value = AutonomousRetryState(failure = failure, sourceSequence = sequence)
                admitted.set(false)
            }
            // Reply data is deliberately not published. Only the Core stream owns it.
        }
    }
}

/** Optional heavy metadata cannot prevent reading the accepted Core report. */
internal suspend fun readAutonomousRoot(
    read: suspend () -> RootSnapshot,
    onFailure: (Exception) -> Unit,
): RootSnapshot? =
    try {
        read()
    } catch (error: CancellationException) {
        throw error
    } catch (error: Exception) {
        onFailure(error)
        null
    }
