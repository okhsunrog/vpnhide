package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ObservationClock
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch

/**
 * Serializes accepted input events and run publications under the run owner's
 * short lock. No I/O or suspending work runs here. In particular a completed run
 * publishes this packet synchronously before its handle is released.
 */
internal class DiagnosticSnapshotOwner(
    private val lock: Any,
    private val processIdentity: String,
    private val clock: () -> Long = ObservationClock::now,
    private val wallClock: () -> Long = System::currentTimeMillis,
) {
    private var sources = DiagnosticSnapshotSources()
    private val mutable = MutableStateFlow(build(sources, revision = 0))
    val snapshot: StateFlow<DiagnosticSnapshot> = mutable.asStateFlow()

    fun update(change: (DiagnosticSnapshotSources) -> DiagnosticSnapshotSources) {
        synchronized(lock) {
            stage(change)
            publish()
        }
    }

    /** Used only inside a domain transition sharing [lock], before its run effects publish. */
    fun stage(change: (DiagnosticSnapshotSources) -> DiagnosticSnapshotSources) {
        synchronized(lock) { sources = change(sources) }
    }

    fun publish() {
        synchronized(lock) {
            val previous = mutable.value
            if (sources != previous.sources) mutable.value = build(sources, previous.revision + 1)
        }
    }

    /** Accept each source in order, including its initial value before returning. */
    fun <T> watch(
        scope: CoroutineScope,
        source: StateFlow<T>,
        change: (DiagnosticSnapshotSources, T) -> DiagnosticSnapshotSources,
    ) {
        scope.launch(start = CoroutineStart.UNDISPATCHED) {
            source.collect { value -> update { change(it, value) } }
        }
    }

    fun watchGrace(
        scope: CoroutineScope,
        wait: suspend (Long) -> Unit = { delay(it) },
    ) {
        scope.launch {
            snapshot.collectLatest { captured ->
                val remaining = backgroundGraceRemaining(captured.presentation, clock())
                if (remaining > 0) {
                    wait(remaining)
                    synchronized(lock) {
                        // A timer for an older revision can never overwrite a new observation.
                        if (mutable.value.revision == captured.revision) {
                            val next = build(captured.sources, captured.revision + 1)
                            if (next.situation != captured.situation) mutable.value = next
                        }
                    }
                }
            }
        }
    }

    private fun build(
        sources: DiagnosticSnapshotSources,
        revision: Long,
    ): DiagnosticSnapshot = diagnosticSnapshot(sources, processIdentity, revision, clock(), wallClock())
}
