package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ContextStateCache
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.ObservationRequest
import dev.okhsunrog.vpnhide.ObservationState
import dev.okhsunrog.vpnhide.ObservedValue
import dev.okhsunrog.vpnhide.ProjectedStateFlow
import dev.okhsunrog.vpnhide.ReadReason
import dev.okhsunrog.vpnhide.checks.AppVpnState
import dev.okhsunrog.vpnhide.checks.AppVpnStateObservation
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext

private const val NEGATIVE_CONFIRM_DELAY_MS = 750L
private const val MAX_SETTLING_SAMPLES = 4

internal data class AppVpnStateSnapshot(
    val gate: DiagnosticGate,
    val session: String?,
    val interfaces: List<String>,
) {
    val identity: String =
        "session=${session ?: "none"};vpn=${interfaces.sorted().joinToString(",")};self=${gate.name}"
}

/**
 * Shared app-scoped VPN-state source. One privileged Rust helper observes
 * VPN presence and this UID's membership directly; the heavy root snapshot is
 * not a prerequisite and no global route/rule fingerprint drives correctness.
 * Negative facts are confirmed twice inside one identified load so a tunnel's
 * brief setup window is published as Checking, not as a false exclusion.
 */
internal object RoutingGateCache : ContextStateCache<AppVpnStateSnapshot>(
    traceName = "routing_gate",
    logTag = LogTags.DIAG,
) {
    val gate: StateFlow<DiagnosticGate?> by lazy { ProjectedStateFlow(current) { it?.gate } }
    val gateObservation: StateFlow<ObservationState<DiagnosticGate>> by lazy {
        ProjectedStateFlow(observation) { it.gateProjection() }
    }

    /** An Activity return always asks for present-tense app VPN state. */
    fun refreshOnResume() {
        forceRefresh(ReadReason.Background)
    }

    /**
     * Probe without invalidating the published observation. The foreground timer
     * must not turn a stable routed value into Checking on every iteration.
     */
    suspend fun observedStateChanged(): Boolean {
        val retainedInputs = inputs ?: return false
        val before = observation.value
        val published = before.lastGood?.value
        if (!before.attempted || before.active != null || before.quarantined || published?.gate == DiagnosticGate.NEEDS_RESTART) {
            return false
        }
        val observed =
            withContext(Dispatchers.IO) { GroundTruthProbe.observeAppVpnState(retainedInputs.context) }
                ?.toSnapshotOrNull()
        val after = observation.value
        if (inputs != retainedInputs || !after.attempted || after.active != null || after.quarantined) return false
        return appVpnStateNeedsRefresh(
            published = after.lastGood?.value,
            observed = observed,
            stale = after.stale != null,
            failed = after.error != null,
        )
    }

    override suspend fun load(
        @Suppress("UNUSED_PARAMETER") request: ObservationRequest,
    ): AppVpnStateSnapshot =
        withContext(Dispatchers.IO) {
            val (context, selfNeedsRestart) = requireNotNull(inputs)
            if (selfNeedsRestart) {
                return@withContext AppVpnStateSnapshot(DiagnosticGate.NEEDS_RESTART, null, emptyList())
            }
            var candidate = requireObservation(GroundTruthProbe.observeAppVpnState(context))
            repeat(MAX_SETTLING_SAMPLES - 1) {
                candidate.validate()
                if (candidate.state == AppVpnState.ROUTED) return@withContext candidate.toSnapshot()
                delay(NEGATIVE_CONFIRM_DELAY_MS)
                val next = requireObservation(GroundTruthProbe.observeAppVpnState(context))
                next.validate()
                if (next.state == AppVpnState.ROUTED) return@withContext next.toSnapshot()
                if (candidate.state == next.state && candidate.session == next.session) {
                    return@withContext next.toSnapshot()
                }
                candidate = next
            }
            error("app VPN state did not settle")
        }

    private fun requireObservation(observation: AppVpnStateObservation?): AppVpnStateObservation =
        requireNotNull(observation) { "app VPN state observation unavailable" }

    private fun AppVpnStateObservation.validate() {
        if (state == AppVpnState.UNKNOWN) error("app VPN state unavailable: $detail")
    }

    private fun AppVpnStateObservation.toSnapshotOrNull(): AppVpnStateSnapshot? = takeUnless { state == AppVpnState.UNKNOWN }?.toSnapshot()

    private fun AppVpnStateObservation.toSnapshot(): AppVpnStateSnapshot =
        AppVpnStateSnapshot(
            gate =
                when (state) {
                    AppVpnState.VPN_OFF -> DiagnosticGate.VPN_OFF
                    AppVpnState.EXCLUDED -> DiagnosticGate.SELF_NOT_ROUTED
                    AppVpnState.ROUTED -> DiagnosticGate.ROUTED
                    AppVpnState.UNKNOWN -> error("app VPN state unavailable: $detail")
                },
            session = session,
            interfaces = interfaces.distinct().sorted(),
        )
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
