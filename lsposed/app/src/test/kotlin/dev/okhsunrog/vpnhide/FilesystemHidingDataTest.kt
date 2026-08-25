package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.settings.FilesystemHidingStatus
import dev.okhsunrog.vpnhide.settings.installedZygiskOptionalHooks
import dev.okhsunrog.vpnhide.settings.resolveFilesystemHidingState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class FilesystemHidingDataTest {
    @Test
    fun `filesystem hiding remains opt in by default`() {
        assertFalse(
            OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS in
                CanonicalSettings().optionalFeatures,
        )
    }

    @Test
    fun `feature is unavailable without a native backend`() {
        assertEquals(
            FilesystemHidingStatus.Unavailable,
            resolveFilesystemHidingState(desiredEnabled = false, sections = emptyMap()).status,
        )
    }

    @Test
    fun `KPM owns the same reboot gated feature`() {
        assertEquals(
            FilesystemHidingStatus.Active,
            resolveFilesystemHidingState(
                desiredEnabled = true,
                sections = sections(hookInstalled = true, backend = HookIds.Backend.KPM),
            ).status,
        )
        assertEquals(
            FilesystemHidingStatus.PendingEnable,
            resolveFilesystemHidingState(
                desiredEnabled = true,
                sections = sections(backend = HookIds.Backend.KPM),
            ).status,
        )
    }

    @Test
    fun `active KPM status wins over an installed inactive kmod`() {
        val sections =
            sections(hookInstalled = true, backend = HookIds.Backend.KPM) +
                ("kmod_module_dir" to "1")

        assertEquals(
            FilesystemHidingStatus.Active,
            resolveFilesystemHidingState(desiredEnabled = true, sections = sections).status,
        )
    }

    @Test
    fun `KPM partial hook setup is reported as an error`() {
        assertEquals(
            FilesystemHidingStatus.HookSetupError,
            resolveFilesystemHidingState(
                desiredEnabled = true,
                sections =
                    sections(
                        filesystemRequested = true,
                        moduleLoaded = true,
                        backend = HookIds.Backend.KPM,
                        statusError = HookIds.StatusError.PARTIAL_HOOKS,
                    ),
            ).status,
        )
    }

    @Test
    fun `Zygisk heartbeat reports active optional filesystem hooks`() {
        val sections =
            zygiskSections(
                requested = HookIds.Hook.FILESYSTEM_IFACE_PATHS.bit,
                installed = HookIds.Hook.FILESYSTEM_IFACE_PATHS.bit,
            )

        assertEquals(
            FilesystemHidingStatus.Active,
            resolveFilesystemHidingState(desiredEnabled = true, sections = sections).status,
        )
        assertEquals(
            setOf(HookIds.Hook.FILESYSTEM_IFACE_PATHS),
            installedZygiskOptionalHooks(sections.getValue("zygisk_status"), "test-boot"),
        )
    }

    @Test
    fun `active Zygisk heartbeat wins over an installed inactive kmod`() {
        val sections =
            zygiskSections(
                requested = HookIds.Hook.FILESYSTEM_IFACE_PATHS.bit,
                installed = HookIds.Hook.FILESYSTEM_IFACE_PATHS.bit,
            ) + ("kmod_module_dir" to "1")

        val state = resolveFilesystemHidingState(desiredEnabled = true, sections = sections)
        assertEquals(FilesystemHidingStatus.Active, state.status)
        assertEquals(NativeBackendId.Zygisk, state.backend)
    }

    @Test
    fun `Zygisk optional hook failure is reported without claiming ownership`() {
        val sections =
            zygiskSections(
                requested = HookIds.Hook.FILESYSTEM_IFACE_PATHS.bit,
                installed = 0,
                error = "shadowhook failed",
            )
        val state = resolveFilesystemHidingState(desiredEnabled = true, sections = sections)

        assertEquals(FilesystemHidingStatus.HookSetupError, state.status)
        assertEquals(NativeBackendId.Zygisk, state.backend)
        assertEquals("shadowhook failed", state.errorDetail)
        assertEquals(
            emptySet<HookIds.Hook>(),
            installedZygiskOptionalHooks(sections.getValue("zygisk_status"), "test-boot"),
        )
    }

    @Test
    fun `stale Zygisk heartbeat leaves an enabled feature pending`() {
        val sections =
            zygiskSections(
                requested = HookIds.Hook.FILESYSTEM_IFACE_PATHS.bit,
                installed = HookIds.Hook.FILESYSTEM_IFACE_PATHS.bit,
                bootId = "old-boot",
            )

        assertEquals(
            FilesystemHidingStatus.PendingEnable,
            resolveFilesystemHidingState(desiredEnabled = true, sections = sections).status,
        )
    }

    @Test
    fun `desired and runtime states distinguish both reboot directions`() {
        assertEquals(
            FilesystemHidingStatus.PendingEnable,
            resolveFilesystemHidingState(desiredEnabled = true, sections = sections()).status,
        )
        assertEquals(
            FilesystemHidingStatus.PendingDisable,
            resolveFilesystemHidingState(
                desiredEnabled = false,
                sections = sections(hookInstalled = true),
            ).status,
        )
        assertEquals(
            FilesystemHidingStatus.Active,
            resolveFilesystemHidingState(
                desiredEnabled = true,
                sections = sections(hookInstalled = true),
            ).status,
        )
    }

    @Test
    fun `fresh boot config errors retain their detail`() {
        val state =
            resolveFilesystemHidingState(
                desiredEnabled = true,
                sections = sections(configExit = 2, configError = "invalid canonical config"),
            )

        assertEquals(FilesystemHidingStatus.BootConfigError, state.status)
        assertEquals("invalid canonical config", state.errorDetail)
    }

    @Test
    fun `requested hook missing after a successful module load is a setup error`() {
        assertEquals(
            FilesystemHidingStatus.HookSetupError,
            resolveFilesystemHidingState(
                desiredEnabled = true,
                sections = sections(filesystemRequested = true, moduleLoaded = true),
            ).status,
        )
    }

    @Test
    fun `stale boot diagnostics do not override current state`() {
        val stale =
            sections(configExit = 2, configError = "old failure") +
                ("current_boot_id" to "new-boot")

        assertEquals(
            FilesystemHidingStatus.Disabled,
            resolveFilesystemHidingState(desiredEnabled = false, sections = stale).status,
        )
    }

    private fun sections(
        hookInstalled: Boolean = false,
        filesystemRequested: Boolean = false,
        moduleLoaded: Boolean = false,
        configExit: Int = if (filesystemRequested) 0 else 1,
        configError: String = "",
        backend: HookIds.Backend = HookIds.Backend.KMOD,
        statusError: HookIds.StatusError = HookIds.StatusError.OK,
    ): Map<String, String> {
        val hookMask =
            if (hookInstalled) {
                setOf(HookIds.Hook.FILESYSTEM_IFACE_PATHS).toHookMask()
            } else {
                0L
            }
        val status =
            Protocol.formatStatus(
                Protocol.Status(
                    backend =
                        backend.id
                            .toLong(),
                    kver = 0,
                    hooks = hookMask,
                    error =
                        statusError.code
                            .toLong(),
                ),
            )
        val loadStatus =
            """
            boot_id=test-boot
            filesystem_hiding=${if (filesystemRequested) 1 else 0}
            filesystem_config_exit=$configExit
            filesystem_config_error=$configError
            loaded=${if (moduleLoaded) 1 else 0}
            """.trimIndent()
        val loadStatusKey = if (backend == HookIds.Backend.KPM) "kpm_load_status" else "kmod_load_status"
        return mapOf(
            (if (backend == HookIds.Backend.KPM) "kpm_module_dir" else "kmod_module_dir") to "1",
            (if (backend == HookIds.Backend.KPM) "kpm_state" else "kmod_state") to status,
            "current_boot_id" to "test-boot",
            loadStatusKey to loadStatus,
        )
    }

    private fun zygiskSections(
        requested: Long,
        installed: Long,
        error: String = "",
        bootId: String = "test-boot",
    ): Map<String, String> =
        mapOf(
            "zygisk_module_dir" to "1",
            "current_boot_id" to "test-boot",
            "zygisk_status" to
                """
                boot_id=$bootId
                requested_hooks=${requested.toString(16)}
                installed_hooks=${installed.toString(16)}
                filesystem_error=$error
                """.trimIndent(),
        )
}
