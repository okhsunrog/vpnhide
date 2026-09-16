package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class CanonicalConfigRepositoryTest {
    @Test
    fun `persistence command orders write coupled state and activators`() {
        val mutation =
            CanonicalMutation(
                emptyList(),
                coupledCommands = listOf("write-secret"),
                activation = CanonicalActivation(native = true, ports = true),
            )

        assertEquals(
            listOf(ConfigPhase.Persist, ConfigPhase.Secret, ConfigPhase.Native, ConfigPhase.Ports),
            configMutationPlan(mutation, true),
        )
        assertTrue(configPhaseCommand(ConfigPhase.Persist, CanonicalConfig(debug = true), mutation).contains(CANONICAL_CONFIG_FILE))
        assertEquals("write-secret", configPhaseCommand(ConfigPhase.Secret, CanonicalConfig(), mutation))
    }

    @Test
    fun `activation can be disabled for settings-only writes`() {
        val mutation =
            CanonicalMutation(
                emptyList(),
                activation = CanonicalActivation(native = false, ports = false),
            )

        assertEquals(listOf(ConfigPhase.Persist), configMutationPlan(mutation, true))
    }

    @Test
    fun `native activator stderr is folded into save output`() {
        val command = ConfigChannels.nativeActivatorCommand()

        assertTrue(command.contains("\"${'$'}ACTIVATOR_PATH\" 2>&1"))
        assertTrue(command.contains("run_activator $KMOD_ACTIVATOR kmod"))
        assertTrue(command.contains("run_activator $KPM_ACTIVATOR KPM"))
        assertTrue(command.contains("run_activator $ZYGISK_ACTIVATOR Zygisk"))
    }

    @Test
    fun `activation selects installed backend before validating its activator`() {
        val command = ConfigChannels.nativeActivatorCommand()

        assertTrue(command.contains("[ -f $KMOD_MODULE_DIR/module.prop ]"))
        assertTrue(command.contains("run_activator $KMOD_ACTIVATOR kmod"))
        assertTrue(command.indexOf("$KMOD_MODULE_DIR/module.prop") < command.indexOf("$KPM_MODULE_DIR/module.prop"))
        assertTrue(command.contains("return 1"))
    }

    @Test
    fun `activator shell helper fails for a corrupted bundle`() {
        val missing = File(System.getProperty("java.io.tmpdir"), "vpnhide-missing-activator-${System.nanoTime()}")
        val command = "${ConfigChannels.activatorShellHelper()}; run_activator ${missing.absolutePath} Test"
        val process = ProcessBuilder("sh", "-c", command).start()
        val stderr = process.errorStream.bufferedReader().readText()

        assertEquals(1, process.waitFor())
        assertTrue(stderr.contains("Test activator missing or not executable at ${missing.absolutePath}"))
    }
}
