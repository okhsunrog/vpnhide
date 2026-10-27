package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.settings.FilesystemHidingStatus
import dev.okhsunrog.vpnhide.settings.installedZygiskOptionalHooks
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

private fun resolveFixtureFilesystemHidingState(
    desiredEnabled: Boolean,
    sections: Map<String, String>,
) = dev.okhsunrog.vpnhide.settings
    .resolveFilesystemHidingState(desiredEnabled, sections, nativeBootStatus(sections))

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
            resolveFixtureFilesystemHidingState(desiredEnabled = false, sections = emptyMap()).status,
        )
    }

    @Test
    fun `KPM owns the same reboot gated feature`() {
        assertEquals(
            FilesystemHidingStatus.Active,
            resolveFixtureFilesystemHidingState(
                desiredEnabled = true,
                sections = sections(hookInstalled = true, backend = HookIds.Backend.KPM),
            ).status,
        )
        assertEquals(
            FilesystemHidingStatus.PendingEnable,
            resolveFixtureFilesystemHidingState(
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
            resolveFixtureFilesystemHidingState(desiredEnabled = true, sections = sections).status,
        )
    }

    @Test
    fun `KPM partial hook setup is reported as an error`() {
        assertEquals(
            FilesystemHidingStatus.HookSetupError,
            resolveFixtureFilesystemHidingState(
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
            resolveFixtureFilesystemHidingState(desiredEnabled = true, sections = sections).status,
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

        val state = resolveFixtureFilesystemHidingState(desiredEnabled = true, sections = sections)
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
        val state = resolveFixtureFilesystemHidingState(desiredEnabled = true, sections = sections)

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
            resolveFixtureFilesystemHidingState(desiredEnabled = true, sections = sections).status,
        )
    }

    @Test
    fun `desired and runtime states distinguish both reboot directions`() {
        assertEquals(
            FilesystemHidingStatus.PendingEnable,
            resolveFixtureFilesystemHidingState(desiredEnabled = true, sections = sections()).status,
        )
        assertEquals(
            FilesystemHidingStatus.PendingDisable,
            resolveFixtureFilesystemHidingState(
                desiredEnabled = false,
                sections = sections(hookInstalled = true),
            ).status,
        )
        assertEquals(
            FilesystemHidingStatus.Active,
            resolveFixtureFilesystemHidingState(
                desiredEnabled = true,
                sections = sections(hookInstalled = true),
            ).status,
        )
    }

    @Test
    fun `requested hook missing after a successful module load is a setup error`() {
        assertEquals(
            FilesystemHidingStatus.HookSetupError,
            resolveFixtureFilesystemHidingState(
                desiredEnabled = true,
                sections = sections(filesystemRequested = true, moduleLoaded = true),
            ).status,
        )
    }

    @Test
    fun `a selection from an earlier boot is not a setup error`() {
        val stale =
            sections(filesystemRequested = true, moduleLoaded = true) +
                ("current_boot_id" to "new-boot")

        assertEquals(
            FilesystemHidingStatus.PendingEnable,
            resolveFixtureFilesystemHidingState(desiredEnabled = true, sections = stale).status,
        )
    }

    private fun sections(
        hookInstalled: Boolean = false,
        filesystemRequested: Boolean = false,
        moduleLoaded: Boolean = false,
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
        val native = if (backend == HookIds.Backend.KPM) "Kpm" else "Kmod"
        val outcome: Any =
            if (moduleLoaded) {
                JSONObject().put(
                    "Selected",
                    JSONObject().put("backend", native).put("mode", "Loaded").put("filesystem_hiding", filesystemRequested),
                )
            } else {
                "NoBackend"
            }
        val boot =
            JSONObject()
                .put("boot_id", "test-boot")
                .put("decided_epoch", 1)
                .put("decided_at_ms", 1)
                .put("updated_at_ms", 1)
                .put("outcome", outcome)
                .put("installed", JSONArray(listOf(native)))
                .put("live", JSONArray())
                .put("staged", JSONArray())
                .put("zygisk_targets", "NotInstalled")
        return mapOf(
            (if (backend == HookIds.Backend.KPM) "kpm_module_dir" else "kmod_module_dir") to "1",
            (if (backend == HookIds.Backend.KPM) "kpm_state" else "kmod_state") to status,
            "current_boot_id" to "test-boot",
            NATIVE_BOOT_SECTION to boot.toString(),
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
