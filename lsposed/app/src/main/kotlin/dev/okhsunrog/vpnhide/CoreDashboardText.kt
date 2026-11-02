package dev.okhsunrog.vpnhide

import androidx.annotation.StringRes

@StringRes
internal fun coreHealthResource(value: CoreHealth): Int =
    when (value) {
        CoreHealth.Connecting -> R.string.core_health_connecting
        CoreHealth.Unavailable -> R.string.core_health_unavailable
        CoreHealth.Stale -> R.string.core_health_stale
        CoreHealth.ProtocolMismatch -> R.string.core_health_protocolmismatch
        CoreHealth.PermissionDenied -> R.string.core_health_permissiondenied
        CoreHealth.Ready -> R.string.core_health_ready
        CoreHealth.StartupFault -> R.string.core_health_startupfault
        CoreHealth.StorageFault -> R.string.core_health_storagefault
        CoreHealth.Paused -> R.string.core_health_paused
        CoreHealth.ConfigurationRequired -> R.string.core_health_configurationrequired
    }

@StringRes
internal fun corePortsResource(value: CorePortsHealth): Int =
    when (value) {
        CorePortsHealth.Held -> R.string.core_ports_held
        CorePortsHealth.Unknown -> R.string.core_ports_unknown
        CorePortsHealth.Disabled -> R.string.core_ports_disabled
        CorePortsHealth.Applied -> R.string.core_ports_applied
        CorePortsHealth.Pending -> R.string.core_ports_pending
        CorePortsHealth.Failed -> R.string.core_ports_failed
    }
