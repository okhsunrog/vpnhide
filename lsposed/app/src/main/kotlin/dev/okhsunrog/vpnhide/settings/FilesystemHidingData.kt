package dev.okhsunrog.vpnhide.settings

import dev.okhsunrog.vpnhide.NativeBackendId
import dev.okhsunrog.vpnhide.NativeBootStatus
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.hasHook
import dev.okhsunrog.vpnhide.hooksInMask
import dev.okhsunrog.vpnhide.installedHooks
import dev.okhsunrog.vpnhide.parseKeyValueLines
import dev.okhsunrog.vpnhide.statistics.parseProtocolStatusBlock
import dev.okhsunrog.vpnhide.statusError

internal enum class FilesystemHidingStatus {
    Unavailable,
    Unknown,
    Disabled,
    Active,
    PendingEnable,
    PendingDisable,
    HookSetupError,
}

internal data class FilesystemHidingState(
    val status: FilesystemHidingStatus,
    val backend: NativeBackendId? = null,
    val errorDetail: String? = null,
) {
    val nativeBackendInstalled: Boolean
        get() = status != FilesystemHidingStatus.Unavailable
}

/** Core owns backend choice and runtime setup. Drafts do not reconstruct it. */
internal fun resolveFilesystemHidingState(snapshot: dev.okhsunrog.vpnhide.NativeStatusSnapshot): FilesystemHidingState {
    val model = snapshot.model
    val backend = model?.displayed?.backend
    if (!snapshot.current) return FilesystemHidingState(FilesystemHidingStatus.Unknown, backend)
    val status =
        when (model?.filesystem) {
            "unavailable" -> FilesystemHidingStatus.Unavailable
            "disabled" -> FilesystemHidingStatus.Disabled
            "enabled" -> FilesystemHidingStatus.Active
            "pending_enable" -> FilesystemHidingStatus.PendingEnable
            "pending_disable" -> FilesystemHidingStatus.PendingDisable
            "setup_error" -> FilesystemHidingStatus.HookSetupError
            else -> FilesystemHidingStatus.Unknown
        }
    return FilesystemHidingState(status, backend, model?.observationError)
}
