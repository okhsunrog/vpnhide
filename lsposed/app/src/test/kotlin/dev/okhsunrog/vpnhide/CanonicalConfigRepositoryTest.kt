package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreRootEffect
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class CanonicalConfigRepositoryTest {
    @Test
    fun `plan orders persist coupled state and activation, and only coupled phases carry root effects`() {
        val mutation =
            CanonicalMutation(
                emptyList(),
                coupledEffects = listOf(CoreRootEffect.SetSuperkey("write-secret")),
                activation = CanonicalActivation(native = true, ports = true),
            )

        assertEquals(
            listOf(ConfigPhase.Persist, ConfigPhase.Secret, ConfigPhase.Native, ConfigPhase.Ports),
            configMutationPlan(mutation, true),
        )
        // Persist / Native / Ports are typed core edits: the app has no shell write for them.
        listOf(ConfigPhase.Persist, ConfigPhase.Native, ConfigPhase.Ports).forEach {
            assertEquals(emptyList<CoreRootEffect>(), configPhaseEffects(it, mutation))
        }
        assertEquals(listOf(CoreRootEffect.SetSuperkey("write-secret")), configPhaseEffects(ConfigPhase.Secret, mutation))
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
}
