package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.CoreSessions
import dev.okhsunrog.vpnhide.ObservationRuntime
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.RootSnapshotCache
import uniffi.vpnhide_core_ffi.CoreException

/** Process-owned subscription to Core evidence; opening UI/export/agent runs no suite. */
internal object DiagnosticsCache {
    private val facade by lazy {
        AutonomousDiagnostics(
            ObservationRuntime.scope,
            CoreSessions.updates,
            RootSnapshotCache.observation,
            CanonicalConfigRepository.state,
        ) {
            val client =
                CoreSessions.client
                    ?: throw CoreException.Unavailable(
                        CoreSessions.updates.value.connectionGeneration
                            .toULong(),
                        "Core is not attached",
                    )
            client.requestDiagnosticRun(force = true)
        }
    }
    val snapshot get() = facade.snapshot
    val retryState get() = facade.retryState

    suspend fun awaitSnapshot(root: RootSnapshot? = null): AutonomousDiagnosticSnapshot = facade.awaitSnapshot(root)

    fun retry(context: Context) {
        CoreSessions.attach(context.applicationContext)
        facade.retry()
    }
}
