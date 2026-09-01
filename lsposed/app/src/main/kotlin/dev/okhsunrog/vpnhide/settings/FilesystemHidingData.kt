package dev.okhsunrog.vpnhide.settings

import dev.okhsunrog.vpnhide.KmodLoadStatus
import dev.okhsunrog.vpnhide.NativeBackendId
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.hasHook
import dev.okhsunrog.vpnhide.hooksInMask
import dev.okhsunrog.vpnhide.installedHooks
import dev.okhsunrog.vpnhide.parseKeyValueLines
import dev.okhsunrog.vpnhide.parseKpmLoadStatus
import dev.okhsunrog.vpnhide.readKmodLoadStatus
import dev.okhsunrog.vpnhide.statistics.parseProtocolStatusBlock
import dev.okhsunrog.vpnhide.statusError

internal enum class FilesystemHidingStatus {
    Unavailable,
    Disabled,
    Active,
    PendingEnable,
    PendingDisable,
    BootConfigError,
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

private data class FilesystemNativeBackend(
    val id: NativeBackendId,
    val statusRaw: String,
)

private fun filesystemNativeBackend(sections: Map<String, String>): FilesystemNativeBackend? {
    val kmodRaw = sections["kmod_state"].orEmpty()
    val kpmRaw = sections["kpm_state"].orEmpty()
    val zygiskRaw = sections["zygisk_status"].orEmpty()
    val kmodStatus = parseProtocolStatusBlock(kmodRaw)
    val kpmStatus = parseProtocolStatusBlock(kpmRaw)
    val zygiskActive =
        parseKeyValueLines(zygiskRaw)["boot_id"] ==
            sections["current_boot_id"].orEmpty().trim() &&
            sections["current_boot_id"].orEmpty().isNotBlank()
    return when {
        kmodStatus?.backend ==
            HookIds.Backend.KMOD.id
                .toLong() -> {
            FilesystemNativeBackend(NativeBackendId.Kmod, kmodRaw)
        }

        // The in-tree driver shares /proc/vpnhide_ctl (the kmod_state section)
        // with the .ko and is told apart only by the backend id it reports (0x4).
        kmodStatus?.backend ==
            HookIds.Backend.BUILTIN.id
                .toLong() -> {
            FilesystemNativeBackend(NativeBackendId.Builtin, kmodRaw)
        }

        kpmStatus?.backend ==
            HookIds.Backend.KPM.id
                .toLong() -> {
            FilesystemNativeBackend(NativeBackendId.Kpm, kpmRaw)
        }

        zygiskActive -> {
            FilesystemNativeBackend(NativeBackendId.Zygisk, zygiskRaw)
        }

        sections["kmod_module_dir"]?.trim() == "1" -> {
            FilesystemNativeBackend(NativeBackendId.Kmod, kmodRaw)
        }

        sections["builtin_module_dir"]?.trim() == "1" -> {
            FilesystemNativeBackend(NativeBackendId.Builtin, kmodRaw)
        }

        sections["kpm_module_dir"]?.trim() == "1" -> {
            FilesystemNativeBackend(NativeBackendId.Kpm, kpmRaw)
        }

        sections["zygisk_module_dir"]?.trim() == "1" -> {
            FilesystemNativeBackend(NativeBackendId.Zygisk, zygiskRaw)
        }

        else -> {
            null
        }
    }
}

private fun currentKmodLoadStatus(
    backend: FilesystemNativeBackend,
    sections: Map<String, String>,
): KmodLoadStatus? =
    if (backend.id == NativeBackendId.Kmod) {
        readKmodLoadStatus(
            currentBootId = sections["current_boot_id"].orEmpty().trim(),
            raw = sections["kmod_load_status"].orEmpty(),
            dmesgRaw = "",
        )?.takeIf { it.freshForCurrentBoot }
    } else {
        null
    }

private fun filesystemHookSetupFailed(
    backend: FilesystemNativeBackend,
    load: KmodLoadStatus?,
    sections: Map<String, String>,
): Boolean =
    when (backend.id) {
        // The in-tree backend has no kmod load_status with filesystem-config
        // fields (its activator only records liveness), so there is no boot-time
        // setup-failure signal to read — same fail-open as an unavailable one.
        NativeBackendId.Kmod, NativeBackendId.Builtin -> {
            load?.let { it.loaded == true && it.filesystemHiding == true } == true
        }

        NativeBackendId.Kpm -> {
            val kpmLoad = parseKpmLoadStatus(sections["kpm_load_status"].orEmpty())
            kpmLoad.loaded == true &&
                kpmLoad.filesystemHiding == true &&
                kpmLoad.isFreshFor(sections["current_boot_id"].orEmpty()) &&
                parseProtocolStatusBlock(backend.statusRaw)?.statusError == HookIds.StatusError.PARTIAL_HOOKS
        }

        NativeBackendId.Zygisk -> {
            val status = currentZygiskHookStatus(backend.statusRaw, sections["current_boot_id"].orEmpty())
            status != null &&
                status.requested.hasHook(HookIds.Hook.FILESYSTEM_IFACE_PATHS) &&
                !status.installed.hasHook(HookIds.Hook.FILESYSTEM_IFACE_PATHS) &&
                status.filesystemError.isNotBlank()
        }
    }

private data class ZygiskHookStatus(
    val requested: Long,
    val installed: Long,
    val filesystemError: String,
)

private fun currentZygiskHookStatus(
    raw: String,
    currentBootId: String,
): ZygiskHookStatus? {
    val values = parseKeyValueLines(raw)
    if (values["boot_id"] != currentBootId.trim() || currentBootId.isBlank()) return null
    return ZygiskHookStatus(
        requested = values["requested_hooks"]?.toLongOrNull(16) ?: 0,
        installed = values["installed_hooks"]?.toLongOrNull(16) ?: 0,
        filesystemError = values["filesystem_error"].orEmpty(),
    )
}

internal fun installedZygiskOptionalHooks(
    raw: String,
    currentBootId: String,
): Set<HookIds.Hook> =
    currentZygiskHookStatus(raw, currentBootId)
        ?.installed
        ?.let(::hooksInMask)
        ?.intersect(setOf(HookIds.Hook.FILESYSTEM_IFACE_PATHS))
        .orEmpty()

internal fun installedNativeOptionalHooks(
    backend: NativeBackendId?,
    sections: Map<String, String>,
    currentBootId: String,
): Set<HookIds.Hook> =
    when (backend) {
        // Builtin shares /proc/vpnhide_ctl with the .ko, so its installed-hook
        // mask is the same kmod_state section.
        NativeBackendId.Kmod, NativeBackendId.Builtin -> {
            installedHooks(sections["kmod_state"].orEmpty())
        }

        NativeBackendId.Kpm -> {
            installedHooks(sections["kpm_state"].orEmpty())
        }

        NativeBackendId.Zygisk -> {
            installedZygiskOptionalHooks(
                sections["zygisk_status"].orEmpty(),
                currentBootId,
            )
        }

        null -> {
            emptySet()
        }
    }

/**
 * Compare the canonical choice with the hook set actually reported by the
 * active native backend. Kernel backends apply it at boot; Zygisk applies it
 * per target-process launch and reports the installed subset in its heartbeat.
 */
internal fun resolveFilesystemHidingState(
    desiredEnabled: Boolean,
    sections: Map<String, String>,
): FilesystemHidingState {
    val backend =
        filesystemNativeBackend(sections)
            ?: return FilesystemHidingState(FilesystemHidingStatus.Unavailable)
    val hookInstalled =
        when (backend.id) {
            NativeBackendId.Kmod, NativeBackendId.Builtin, NativeBackendId.Kpm -> {
                HookIds.Hook.FILESYSTEM_IFACE_PATHS in installedHooks(backend.statusRaw)
            }

            NativeBackendId.Zygisk -> {
                HookIds.Hook.FILESYSTEM_IFACE_PATHS in
                    installedZygiskOptionalHooks(
                        backend.statusRaw,
                        sections["current_boot_id"].orEmpty(),
                    )
            }
        }
    val load = currentKmodLoadStatus(backend, sections)
    val configExit = load?.filesystemConfigExit
    if (configExit != null && configExit != 0 && configExit != 1) {
        return FilesystemHidingState(
            status = FilesystemHidingStatus.BootConfigError,
            backend = backend.id,
            errorDetail = load.filesystemConfigError ?: "exit=$configExit",
        )
    }

    return when {
        desiredEnabled && hookInstalled -> {
            FilesystemHidingState(FilesystemHidingStatus.Active, backend.id)
        }

        !desiredEnabled && hookInstalled -> {
            FilesystemHidingState(FilesystemHidingStatus.PendingDisable, backend.id)
        }

        desiredEnabled && filesystemHookSetupFailed(backend, load, sections) -> {
            FilesystemHidingState(
                FilesystemHidingStatus.HookSetupError,
                backend.id,
                currentZygiskHookStatus(
                    backend.statusRaw,
                    sections["current_boot_id"].orEmpty(),
                )?.filesystemError,
            )
        }

        desiredEnabled -> {
            FilesystemHidingState(FilesystemHidingStatus.PendingEnable, backend.id)
        }

        else -> {
            FilesystemHidingState(FilesystemHidingStatus.Disabled, backend.id)
        }
    }
}
