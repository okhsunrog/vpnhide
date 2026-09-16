package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CanonicalEditDataTest {
    private val noActivation = CanonicalActivation(native = false)

    @Test
    fun `invalid candidate and unframeable secret reject before any persistence`() {
        val read = RootCanonicalRead.Available(CanonicalConfig())
        val badConfig = CanonicalMutation(listOf(CanonicalEdit.Replace(CanonicalConfig(version = 9))))
        val badSecret =
            CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)), coupledCommands = listOf("bad\u0000command"))
        for (mutation in listOf(badConfig, badSecret)) {
            val result = prepareConfigMutation(EffectTicket(1, 1), read, mutation) as ConfigOperationEvent.PreparationFailed
            assertEquals(TransitionFailure.ValidationFailed, result.reason)
        }
    }

    @Test
    fun `edits from stale editor preserve other roles and settings on fresh config`() {
        val old = CanonicalConfig(apps = mapOf("example.app" to CanonicalApp(java = true)))
        val desired = old.copy(apps = emptyMap(), debugSwitch = true)
        val fresh =
            old.copy(
                apps = mapOf("example.app" to CanonicalApp(java = true, native = NativeRole.All)),
                settings = old.settings.copy(autoHideVpnName = true),
            )
        val result = applyCanonicalMutation(fresh, CanonicalMutation(canonicalEdits(old, desired)))
        assertFalse(result.apps.getValue("example.app").java)
        assertTrue(
            result.apps
                .getValue("example.app")
                .native.enabled,
        )
        assertTrue(result.settings.autoHideVpnName)
        assertTrue(result.debugSwitch)
    }

    @Test
    fun `round trip covers settings hooks policies and explicit removal`() {
        val old = CanonicalConfig(apps = mapOf("gone.app" to CanonicalApp(hidden = true)))
        val next =
            CanonicalConfig(
                debug = true,
                debugSwitch = true,
                settings = CanonicalSettings(true, setOf("feature"), false, true, setOf("excluded.app"), setOf("hidden.app")),
                apps =
                    mapOf(
                        "example.app" to
                            CanonicalApp(
                                java = true,
                                javaHooks = listOf("java_hook"),
                                native = NativeRole(true, NativeHookOverrides(kernel = listOf("native_hook"))),
                                appHiding = true,
                                ports = true,
                                hidden = true,
                            ),
                    ),
            )
        assertEquals(next, applyCanonicalMutation(old, CanonicalMutation(canonicalEdits(old, next))))
        assertTrue(applyCanonicalEdit(next, CanonicalEdit.RemoveApp("example.app")).apps.isEmpty())
        assertEquals(next, applyCanonicalEdit(old, CanonicalEdit.Replace(next)))
    }

    @Test
    fun `submitted values detach mutable collections and secret list`() {
        val hooks = mutableListOf("first")
        val features = mutableSetOf("feature")
        val commands = mutableListOf("private secret")
        val edits =
            mutableListOf<CanonicalEdit>(
                CanonicalEdit.App(
                    "example.app",
                    CanonicalAppField.Native,
                    CanonicalApp(native = NativeRole(true, NativeHookOverrides(kernel = hooks))),
                ),
                CanonicalEdit.SettingSet(CanonicalSetField.OptionalFeatures, features),
            )
        val mutation = CanonicalMutation(edits, coupledCommands = commands)
        hooks.clear()
        features.clear()
        commands.clear()
        edits.clear()
        val actual = applyCanonicalMutation(CanonicalConfig(), mutation)
        assertEquals(
            listOf("first"),
            actual.apps
                .getValue("example.app")
                .native.overrides.kernel,
        )
        assertEquals(setOf("feature"), actual.settings.optionalFeatures)
        assertEquals(listOf("private secret"), mutation.coupledCommands)
        assertFalse(mutation.toString().contains("private secret"))
    }

    @Test
    fun `explicit same value intent remains in write set while no op plan executes nothing`() {
        val mutation = CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, false)))
        val prepared =
            prepareConfigMutation(
                EffectTicket(1, 1),
                RootCanonicalRead.Available(CanonicalConfig()),
                mutation,
            ) as ConfigOperationEvent.Prepared
        assertEquals(emptyList<ConfigPhase>(), prepared.plan)
        assertEquals(setOf(ConfigField(listOf("debugSwitch"))), mutation.writes)
        assertEquals(mapOf(CanonicalToggle.DebugSwitch to false), pendingConfigToggles(listOf(mutation)))
    }

    @Test
    fun `missing canonical is only writable by explicit bootstrap`() {
        val ticket = EffectTicket(1, 1)
        val normal = CanonicalMutation(emptyList(), activation = noActivation)
        assertTrue(prepareConfigMutation(ticket, RootCanonicalRead.Missing, normal) is ConfigOperationEvent.PreparationFailed)
        val bootstrap = CanonicalMutation(emptyList(), activation = noActivation, bootstrap = true)
        val prepared = prepareConfigMutation(ticket, RootCanonicalRead.Missing, bootstrap) as ConfigOperationEvent.Prepared
        assertEquals(listOf(ConfigPhase.Persist), prepared.plan)
        listOf(RootCanonicalRead.Invalid, RootCanonicalRead.Unavailable).forEach {
            assertTrue(prepareConfigMutation(ticket, it, bootstrap) is ConfigOperationEvent.PreparationFailed)
        }
    }

    @Test
    fun `phase plan keeps secret and activation repair separate from persistence`() {
        assertEquals(emptyList<ConfigPhase>(), configMutationPlan(CanonicalMutation(emptyList()), changed = false))
        val repair = CanonicalMutation(emptyList(), activation = CanonicalActivation(ports = true), forceActivation = true)
        assertEquals(listOf(ConfigPhase.Native, ConfigPhase.Ports), configMutationPlan(repair, changed = false))
        val secret = CanonicalMutation(emptyList(), activation = noActivation, coupledCommands = listOf("private"))
        assertEquals(listOf(ConfigPhase.Secret), configMutationPlan(secret, changed = false))
        assertEquals("private", configPhaseCommand(ConfigPhase.Secret, CanonicalConfig(), secret))
        assertEquals(ConfigChannels.nativeActivatorCommand(), configPhaseCommand(ConfigPhase.Native, CanonicalConfig(), repair))
        assertEquals(ConfigChannels.portsActivatorCommand(), configPhaseCommand(ConfigPhase.Ports, CanonicalConfig(), repair))
    }
}
