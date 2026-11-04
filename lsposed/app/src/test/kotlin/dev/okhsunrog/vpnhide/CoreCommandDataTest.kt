package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreRootEffect
import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreCommand

class CoreCommandDataTest {
    @Test
    fun `startup requests Core preparation without supplying a canonical replacement`() {
        val mutation =
            CanonicalMutation(
                emptyList(),
                bootstrap = true,
                protectAllDrafts = true,
                coupledEffects = listOf(CoreRootEffect.PrepareSelfTargets),
            )
        val prepared = prepareConfigIntent(RootCanonicalRead.Missing, mutation) as ConfigIntentPreparation.Prepared
        assertEquals(CoreCommand.PrepareSelfTargets(0u), coreCommand(0, mutation, prepared.intent))
        assertEquals(setOf("apps", "settings", "debug", "debugSwitch"), mutation.writes.map { it.segments.single() }.toSet())
    }

    @Test
    fun `legacy import and cleanup remain one durable command`() {
        val config = CanonicalConfig(apps = mapOf("app" to CanonicalApp(java = true)))
        val mutation = CanonicalMutation(listOf(CanonicalEdit.Replace(config)), coupledEffects = listOf(CoreRootEffect.DeleteLegacyConfig))
        val prepared = prepareConfigIntent(RootCanonicalRead.Available(CanonicalConfig()), mutation) as ConfigIntentPreparation.Prepared
        assertEquals(CoreCommand.Import(7u, config.toCore(), true), coreCommand(7, mutation, prepared.intent))
    }

    @Test(expected = IllegalArgumentException::class)
    fun `secret command cannot silently discard unrelated config intent`() {
        val mutation =
            CanonicalMutation(
                listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)),
                coupledEffects = listOf(CoreRootEffect.SetSuperkey("private")),
            )
        val prepared = prepareConfigIntent(RootCanonicalRead.Available(CanonicalConfig()), mutation) as ConfigIntentPreparation.Prepared
        coreCommand(7, mutation, prepared.intent)
    }

    @Test
    fun `backend choice uses one durable command without canonical edits`() {
        val choice =
            uniffi.vpnhide_core_ffi.CoreBackendChoiceRequest(
                3u,
                uniffi.vpnhide_core_ffi.CoreBackendChoice.Fixed(uniffi.vpnhide_core_ffi.CoreNativeBackend.KMOD),
                null,
            )
        val mutation =
            CanonicalMutation(emptyList(), coupledEffects = listOf(CoreRootEffect.StageBackendChoice(choice)), protectAllDrafts = true)
        val config = CanonicalConfig(apps = mapOf("app" to CanonicalApp(java = true)))
        val prepared = prepareConfigIntent(RootCanonicalRead.Available(config), mutation) as ConfigIntentPreparation.Prepared
        assertEquals(config, prepared.intent.candidate)
        assertEquals(emptyList<CanonicalEdit>(), prepared.intent.edits)
        assertEquals(CoreCommand.StageBackendChoice(7u, choice), coreCommand(7, mutation, prepared.intent))
    }
}
