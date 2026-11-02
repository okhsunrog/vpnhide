package dev.okhsunrog.vpnhide

import kotlinx.serialization.Serializable

@Serializable
internal enum class CoreHealth {
    Connecting,
    Unavailable,
    Stale,
    ProtocolMismatch,
    PermissionDenied,
    Ready,
    StartupFault,
    StorageFault,
    Paused,
    ConfigurationRequired,
}

@Serializable
internal enum class CorePortsHealth { Unknown, Disabled, Applied, Pending, Failed, Held }

/** Component health is independent of Ports policy and VPN-hiding measurements. */
@Serializable
internal data class CoreDashboardData(
    val health: CoreHealth = CoreHealth.Connecting,
    val installedVersion: String? = null,
    val detail: String? = null,
    val ports: CorePortsHealth = CorePortsHealth.Unknown,
    val portsTargets: Int? = null,
)

internal fun coreDashboardData(
    update: CoreSessionUpdate,
    installedVersion: String? = null,
): CoreDashboardData {
    val state = update.state.takeIf { update.current }
    val config = (state?.canonical as? RootCanonicalRead.Available)?.config
    val targets = config?.apps?.values?.count { it.ports }
    val evidence = state?.applyEvidence?.lastOrNull { it.phase == ConfigPhase.Ports && it.desiredRevision == state.configRevision }
    val ports =
        when {
            targets == null -> CorePortsHealth.Unknown
            ConfigPhase.Ports in state.heldResources -> CorePortsHealth.Held
            evidence?.outcome == PhaseOutcome.FailedKnown -> CorePortsHealth.Failed
            evidence?.outcome == PhaseOutcome.Confirmed -> if (targets == 0) CorePortsHealth.Disabled else CorePortsHealth.Applied
            evidence?.outcome == PhaseOutcome.Running -> CorePortsHealth.Pending
            else -> CorePortsHealth.Unknown
        }
    return CoreDashboardData(
        coreHealth(update),
        installedVersion,
        (state?.storageFault ?: state?.startupError ?: update.failureDetail)?.take(512),
        ports,
        targets,
    )
}

private fun coreHealth(update: CoreSessionUpdate): CoreHealth {
    val state = update.state.takeIf { update.current }
    return when {
        update.protocolSkew -> CoreHealth.ProtocolMismatch

        !update.current && update.failureDetail?.let {
            it.contains("Permission denied", true) || it.contains("os error 13")
        } == true -> CoreHealth.PermissionDenied

        !update.current && update.state != null -> CoreHealth.Stale

        !update.current && update.sequence > 0 -> CoreHealth.Unavailable

        state == null -> CoreHealth.Connecting

        state.storageFault != null -> CoreHealth.StorageFault

        state.startupError != null -> CoreHealth.StartupFault

        state.mutationHeld || state.heldResources.isNotEmpty() -> CoreHealth.Paused

        state.canonical !is RootCanonicalRead.Available -> CoreHealth.ConfigurationRequired

        else -> CoreHealth.Ready
    }
}
