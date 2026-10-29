package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreRootEffect
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreRecoveryAccess

class RecoveryAccessDataTest {
    private val access = CoreRecoveryAccess(null, null, null, null, null)

    @Test
    fun `recovery permits explicit actions without reopening ordinary edits`() {
        assertTrue(recoveryAllowed(access, CanonicalMutation(emptyList(), forceActivation = true)))
        assertTrue(recoveryAllowed(access, CanonicalMutation(emptyList(), coupledEffects = listOf(CoreRootEffect.ClearSuperkey))))
        assertTrue(
            recoveryAllowed(
                access,
                CanonicalMutation(emptyList(), coupledEffects = listOf(CoreRootEffect.FullReset), removesCanonical = true),
            ),
        )
        assertTrue(recoveryAllowed(access, CanonicalMutation(emptyList(), coupledEffects = listOf(CoreRootEffect.DeleteLegacyConfig))))
        assertFalse(recoveryAllowed(access, CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.Debug, true)))))
        assertFalse(
            recoveryAllowed(
                access,
                CanonicalMutation(emptyList(), protectAllDrafts = true, coupledEffects = listOf(CoreRootEffect.DeleteLegacyConfig)),
            ),
        )
    }

    @Test
    fun `missing evidence and persist uncertainty never allow credentials`() {
        val mutation = CanonicalMutation(emptyList(), coupledEffects = listOf(CoreRootEffect.SetSuperkey("test")))
        assertFalse(recoveryAllowed(null, mutation))
        assertFalse(recoveryAllowed(access.copy(setSuperkey = "persist_uncertain"), mutation))
        assertTrue(recoveryAllowed(access, mutation))
    }
}
