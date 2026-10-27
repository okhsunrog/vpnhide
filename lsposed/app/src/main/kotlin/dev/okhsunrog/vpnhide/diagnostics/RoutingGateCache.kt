package dev.okhsunrog.vpnhide.diagnostics

import android.os.Process
import dev.okhsunrog.vpnhide.ContextStateCache
import dev.okhsunrog.vpnhide.CoreSessionUpdate
import dev.okhsunrog.vpnhide.CoreSessions
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.ObservationDependency
import dev.okhsunrog.vpnhide.ObservationRequest
import dev.okhsunrog.vpnhide.ObservationRuntime
import dev.okhsunrog.vpnhide.ObservationState
import dev.okhsunrog.vpnhide.ObservedValue
import dev.okhsunrog.vpnhide.ProjectedStateFlow
import dev.okhsunrog.vpnhide.ReadReason
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

private const val NEGATIVE_CONFIRM_DELAY_MS = 750L

internal data class AppVpnStateSnapshot(
    val gate: DiagnosticGate,
    val session: String?,
    val interfaces: List<String>,
) {
    val identity: String = "session=${session ?: "none"};vpn=${interfaces.sorted().joinToString(",")};self=${gate.name}"
}

/** Root UID observations arrive over the process session even when Android callbacks are hidden. */
private object CoreRoutingDependency : ObservationDependency {
    override fun subscribe(invalidate: () -> Unit) {
        ObservationRuntime.scope.launch {
            CoreSessions.updates
                .map { coreRoutingSnapshot(it, Process.myUid()) }
                .distinctUntilChanged()
                .collect { invalidate() }
        }
    }

    // The continuously subscribed source owns refresh/resync. No second per-app probe or timer.
    override fun refresh() = Unit
}

internal object RoutingGateCache : ContextStateCache<AppVpnStateSnapshot>(
    traceName = "routing_gate",
    logTag = LogTags.DIAG,
    source = CoreRoutingDependency,
) {
    val gate: StateFlow<DiagnosticGate?> by lazy { ProjectedStateFlow(current) { it?.gate } }
    val gateObservation: StateFlow<ObservationState<DiagnosticGate>> by lazy { ProjectedStateFlow(observation) { it.gateProjection() } }

    fun refreshOnResume() {
        forceRefresh(ReadReason.Background)
    }

    override suspend fun load(
        @Suppress("UNUSED_PARAMETER") request: ObservationRequest,
    ): AppVpnStateSnapshot {
        if (requireNotNull(inputs).selfNeedsRestart) return AppVpnStateSnapshot(DiagnosticGate.NEEDS_RESTART, null, emptyList())
        val candidate =
            requireNotNull(coreRoutingSnapshot(CoreSessions.updates.value, Process.myUid())) { "core routing observation unavailable" }
        if (candidate.gate == DiagnosticGate.ROUTED) return candidate
        // A transient tunnel setup window remains Checking until the negative fact is stable.
        delay(NEGATIVE_CONFIRM_DELAY_MS)
        val next =
            requireNotNull(coreRoutingSnapshot(CoreSessions.updates.value, Process.myUid())) { "core routing observation unavailable" }
        require(next == candidate || next.gate == DiagnosticGate.ROUTED) { "core routing observation changed while settling" }
        return next
    }
}

/** Last-good routing is display-only; disconnect, wrong UID and Unknown invalidate readiness. */
internal fun coreRoutingSnapshot(
    update: CoreSessionUpdate,
    expectedUid: Int,
): AppVpnStateSnapshot? {
    if (!update.current) return null
    val routing = update.state?.routing ?: return null
    if (!routing.current || routing.uid != expectedUid) return null
    val gate =
        when (routing.state) {
            uniffi.vpnhide_core_ffi.CoreUidVpnRouteState.VPN_OFF -> DiagnosticGate.VPN_OFF
            uniffi.vpnhide_core_ffi.CoreUidVpnRouteState.EXCLUDED -> DiagnosticGate.SELF_NOT_ROUTED
            uniffi.vpnhide_core_ffi.CoreUidVpnRouteState.ROUTED -> DiagnosticGate.ROUTED
            else -> return null
        }
    return AppVpnStateSnapshot(gate, routing.session, routing.interfaces)
}

internal fun ObservationState<AppVpnStateSnapshot>.gateProjection(): ObservationState<DiagnosticGate> =
    ObservationState(
        lastGood = lastGood?.let { ObservedValue(it.value.gate, it.request, it.finishedAt) },
        active = active,
        generation = generation,
        stale = stale,
        nextId = nextId,
        error = error,
        attempted = attempted,
        attemptedGeneration = attemptedGeneration,
        quarantined = quarantined,
        quarantineRequestId = quarantineRequestId,
    )
