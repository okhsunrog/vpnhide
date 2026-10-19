package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ModuleIntegrityDataTest {
    private val installed = ModuleState.Installed(version = "1.0.0", active = false)

    @Test
    fun `artifact state parser keeps integrity outcomes typed`() {
        assertEquals(ArtifactState.Present, parseArtifactState("present\n"))
        assertEquals(ArtifactState.Missing, parseArtifactState("missing"))
        assertEquals(ArtifactState.Unknown, parseArtifactState("executable"))
        assertEquals(ArtifactState.Unknown, parseArtifactState(""))
    }

    @Test
    fun `enabled installed module reports its missing payload`() {
        val problem =
            classifyModuleIntegrity(
                kind = FlashableModuleKind.Zygisk,
                module = installed,
                artifactState = ArtifactState.Missing,
                disabled = false,
            )

        assertEquals(ModuleBrokenReason.ArtifactMissing, problem?.reason)
        assertEquals("$ZYGISK_MODULE_DIR/zygisk/", problem?.artifactPath)
    }

    @Test
    fun `modules without a payload of their own never report corruption`() {
        for (kind in listOf(FlashableModuleKind.Builtin, FlashableModuleKind.Core)) {
            assertNull(classifyModuleIntegrity(kind, installed, ArtifactState.Missing, disabled = false))
        }
    }

    @Test
    fun `disabled absent healthy and unknown modules do not report corruption`() {
        assertNull(classify(ArtifactState.Missing, disabled = true, module = installed))
        assertNull(classify(ArtifactState.Missing, disabled = false, module = ModuleState.NotInstalled))
        assertNull(classify(ArtifactState.Present, disabled = false, module = installed))
        assertNull(classify(ArtifactState.Unknown, disabled = false, module = installed))
    }

    @Test
    fun `missing payload with a staged install is pending reboot, not corruption`() {
        val sections = sectionsOf(artifactState = "missing", disabled = "0", pendingUpdate = "1")

        assertTrue(modulePendingReboot(FlashableModuleKind.Kpm, installed, sections))
        // The integrity error is suppressed so the UI shows a reboot warning instead.
        assertNull(moduleIntegrityProblem(FlashableModuleKind.Kpm, installed, sections))
    }

    @Test
    fun `missing payload without a staged install stays a corruption error`() {
        val sections = sectionsOf(artifactState = "missing", disabled = "0", pendingUpdate = "0")

        assertFalse(modulePendingReboot(FlashableModuleKind.Kpm, installed, sections))
        val problem = moduleIntegrityProblem(FlashableModuleKind.Kpm, installed, sections)
        assertEquals(ModuleBrokenReason.ArtifactMissing, problem?.reason)
        assertEquals("$KPM_MODULE_DIR/vpnhide.kpm", problem?.artifactPath)
    }

    @Test
    fun `a staged update needs an enabled installed module to be pending reboot`() {
        // A healthy module with an update staged still needs the reboot.
        assertTrue(
            modulePendingReboot(
                FlashableModuleKind.Core,
                installed,
                mapOf("core_disabled" to "0", "core_pending_update" to "1"),
            ),
        )
        assertFalse(
            modulePendingReboot(
                FlashableModuleKind.Kpm,
                installed,
                sectionsOf(artifactState = "missing", disabled = "1", pendingUpdate = "1"),
            ),
        )
        assertFalse(
            modulePendingReboot(
                FlashableModuleKind.Kpm,
                ModuleState.NotInstalled,
                sectionsOf(artifactState = "missing", disabled = "0", pendingUpdate = "1"),
            ),
        )
        // Absent snapshot field defaults to not-pending.
        assertFalse(
            modulePendingReboot(
                FlashableModuleKind.Kpm,
                installed,
                mapOf("kpm_artifact_state" to "missing", "kpm_disabled" to "0"),
            ),
        )
        // The in-tree backend has no module to stage.
        assertFalse(modulePendingReboot(FlashableModuleKind.Builtin, installed, mapOf("builtin_pending_update" to "1")))
    }

    private fun sectionsOf(
        artifactState: String,
        disabled: String,
        pendingUpdate: String,
    ): Map<String, String> =
        mapOf(
            "kpm_artifact_state" to artifactState,
            "kpm_disabled" to disabled,
            "kpm_pending_update" to pendingUpdate,
        )

    private fun classify(
        state: ArtifactState,
        disabled: Boolean,
        module: ModuleState,
    ): ModuleIntegrityProblem? =
        classifyModuleIntegrity(
            kind = FlashableModuleKind.Kpm,
            module = module,
            artifactState = state,
            disabled = disabled,
        )
}
