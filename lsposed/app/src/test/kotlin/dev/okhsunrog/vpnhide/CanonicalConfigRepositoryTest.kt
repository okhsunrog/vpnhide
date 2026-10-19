package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class CanonicalConfigRepositoryTest {
    @Test
    fun `plan orders persist coupled state and activation, and only coupled phases carry shell`() {
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
        // Persist / Native / Ports are typed core edits: the app has no shell write for them.
        listOf(ConfigPhase.Persist, ConfigPhase.Native, ConfigPhase.Ports).forEach {
            assertEquals("", configPhaseCommand(it, mutation))
        }
        assertEquals("write-secret", configPhaseCommand(ConfigPhase.Secret, mutation))
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
    fun `no app-side command names a core-owned path`() {
        val mutation = CanonicalMutation(emptyList(), coupledCommands = listOf(buildSuperkeyClearCommand()))
        ConfigPhase.entries.forEach { phase ->
            val command = configPhaseCommand(phase, mutation)
            listOf(CANONICAL_CONFIG_FILE, "/proc/vpnhide_ctl", "targets.txt", "iptables", "activator").forEach {
                assertTrue("$phase must not reference $it", it !in command)
            }
        }
    }
}
