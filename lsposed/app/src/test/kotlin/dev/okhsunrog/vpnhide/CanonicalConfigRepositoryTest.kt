package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreConfigAppField
import uniffi.vpnhide_core_ffi.CoreEdit
import uniffi.vpnhide_core_ffi.CoreFieldEdit
import uniffi.vpnhide_core_ffi.CorePortPolicyMode

class CanonicalConfigRepositoryTest {
    @Test
    fun `multi field intent is a typed flat batch`() {
        val batch =
            coreEdit(
                listOf(
                    CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true),
                    CanonicalEdit.App("example.app", CanonicalAppField.Native, CanonicalApp(native = NativeRole.All)),
                ),
            ) as CoreEdit.Batch
        assertEquals(2, batch.edits.size)
        assertTrue((batch.edits.first() as CoreFieldEdit.Toggle).enabled)
        val app = batch.edits.last() as CoreFieldEdit.App
        assertEquals(CoreConfigAppField.NATIVE, app.field)
        assertTrue(app.value.native.enabled)
    }

    @Test
    fun `typed config round trip preserves explicit empty masks sets and port policy`() {
        val config =
            CanonicalConfig(
                debug = true,
                debugSwitch = true,
                settings = CanonicalSettings(optionalFeatures = setOf("b", "a")),
                apps =
                    mapOf(
                        "example.app" to
                            CanonicalApp(
                                java = true,
                                javaHooks = emptyList(),
                                native = NativeRole(true, NativeHookOverrides(kernel = emptyList(), zygisk = listOf("open"))),
                                ports = true,
                                portPolicy = PortPolicy(PortPolicyMode.Custom, null, listOf(PortRule(PortProtocol.Tcp, 8080, 8090))),
                            ),
                    ),
            )
        assertEquals(config, config.toCore().toCanonical())
        val replacement = coreEdit(listOf(CanonicalEdit.Replace(config))) as CoreEdit.Replace
        assertEquals(listOf("a", "b"), replacement.config.settings.optionalFeatures)
    }

    @Test(expected = IllegalStateException::class)
    fun `unknown typed port modes are not guessed into a custom policy`() {
        val config =
            CanonicalConfig(
                apps =
                    mapOf(
                        "example.app" to
                            CanonicalApp(
                                ports = true,
                                portPolicy = PortPolicy(PortPolicyMode.Custom, null, listOf(PortRule(start = 8080, end = 8080))),
                            ),
                    ),
            ).toCore()
        config.apps
            .getValue("example.app")
            .portPolicy!!
            .mode = CorePortPolicyMode.Unknown("future")
        config.toCanonical()
    }
}
