package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreRootEffect
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CanonicalEditDataTest {
    @Test
    fun `invalid candidate and unframeable secret reject before any persistence`() {
        val read = RootCanonicalRead.Available(CanonicalConfig())
        val badConfig = CanonicalMutation(listOf(CanonicalEdit.Replace(CanonicalConfig(version = 9))))
        val badSecret =
            CanonicalMutation(
                listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)),
                coupledEffects = listOf(CoreRootEffect.SetSuperkey("bad\u0000key")),
            )
        for (mutation in listOf(badConfig, badSecret)) {
            val result = prepareConfigIntent(read, mutation) as ConfigIntentPreparation.Failed
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
        val commands = mutableListOf<CoreRootEffect>(CoreRootEffect.SetSuperkey("private secret"))
        val edits =
            mutableListOf<CanonicalEdit>(
                CanonicalEdit.App(
                    "example.app",
                    CanonicalAppField.Native,
                    CanonicalApp(native = NativeRole(true, NativeHookOverrides(kernel = hooks))),
                ),
                CanonicalEdit.SettingSet(CanonicalSetField.OptionalFeatures, features),
            )
        val mutation = CanonicalMutation(edits, coupledEffects = commands)
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
        assertEquals(listOf(CoreRootEffect.SetSuperkey("private secret")), mutation.coupledEffects)
        assertFalse(mutation.toString().contains("private secret"))
    }

    @Test
    fun `explicit same value intent remains in write set while no op plan executes nothing`() {
        val mutation = CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, false)))
        val prepared =
            prepareConfigIntent(RootCanonicalRead.Available(CanonicalConfig()), mutation) as ConfigIntentPreparation.Prepared
        assertEquals(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, false)), prepared.intent.edits)
        assertEquals(setOf(ConfigField(listOf("debugSwitch"))), mutation.writes)
        assertEquals(mapOf(CanonicalToggle.DebugSwitch to false), pendingConfigToggles(listOf(mutation)))
    }

    @Test
    fun `missing canonical is only writable by explicit bootstrap`() {
        val normal = CanonicalMutation(emptyList())
        assertTrue(prepareConfigIntent(RootCanonicalRead.Missing, normal) is ConfigIntentPreparation.Failed)
        val bootstrap = CanonicalMutation(emptyList(), bootstrap = true)
        val prepared = prepareConfigIntent(RootCanonicalRead.Missing, bootstrap) as ConfigIntentPreparation.Prepared
        assertTrue(prepared.intent.edits.single() is CanonicalEdit.Replace)
        listOf(RootCanonicalRead.Invalid, RootCanonicalRead.Unavailable).forEach {
            assertTrue(prepareConfigIntent(it, bootstrap) is ConfigIntentPreparation.Failed)
        }
    }

    @Test
    fun `self target policy overrides same value role intent and explicit removal`() {
        val protected = canonicalConfigWithSelfTarget(CanonicalConfig(), "self.app")
        val forbidden = CanonicalEdit.App("self.app", CanonicalAppField.Ports, CanonicalApp(ports = true))
        for (edit in listOf(forbidden, CanonicalEdit.RemoveApp("self.app"))) {
            val mutation = CanonicalMutation(listOf(edit), transform = { canonicalConfigWithSelfTarget(it, "self.app") })
            val preparation = prepareConfigIntent(RootCanonicalRead.Available(protected), mutation) as ConfigIntentPreparation.Prepared
            val applied = preparation.intent.edits.fold(protected, ::applyCanonicalEdit)
            assertEquals(protected, applied)
            assertFalse(applied.apps.getValue("self.app").ports)
        }
    }
}
