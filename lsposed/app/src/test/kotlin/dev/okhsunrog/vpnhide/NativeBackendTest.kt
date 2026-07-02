package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class NativeBackendTest {
    private fun installed(active: Boolean) = ModuleState.Installed(version = "1.0", active = active, targetCount = 1)

    private fun states(
        kmod: ModuleState,
        kpm: ModuleState,
        zygisk: ModuleState,
    ) = NativeBackendStates(kmod = kmod, kpm = kpm, zygisk = zygisk)

    // ── activeNativeBackendId / displayNativeBackend ─────────────────────

    @Test
    fun `no native installed has no active or display backend`() {
        val states = states(ModuleState.NotInstalled, ModuleState.NotInstalled, ModuleState.NotInstalled)

        assertNull(states.activeId)
        assertNull(displayNativeBackend(states).id)
        assertEquals(ModuleState.NotInstalled, displayNativeBackend(states).state)
    }

    @Test
    fun `single installed backend is displayed`() {
        assertEquals(
            NativeBackendId.Kpm,
            displayNativeBackend(states(ModuleState.NotInstalled, installed(active = false), ModuleState.NotInstalled)).id,
        )
        assertEquals(
            NativeBackendId.Zygisk,
            displayNativeBackend(states(ModuleState.NotInstalled, ModuleState.NotInstalled, installed(active = true))).id,
        )
    }

    @Test
    fun `active backend is reported and displayed over higher-priority inactive one`() {
        // kmod installed but inactive, zygisk active -> show the active zygisk.
        val states = states(installed(active = false), ModuleState.NotInstalled, installed(active = true))
        val sel = displayNativeBackend(states)
        assertEquals(NativeBackendId.Zygisk, states.activeId)
        assertEquals(NativeBackendId.Zygisk, sel.id)
        assertEquals(true, moduleActive(sel.state))
    }

    @Test
    fun `with none active display priority order kmod over kpm over zygisk wins`() {
        assertEquals(
            NativeBackendId.Kmod,
            displayNativeBackend(states(installed(active = false), installed(active = false), installed(active = false))).id,
        )
        assertEquals(
            NativeBackendId.Kpm,
            displayNativeBackend(states(ModuleState.NotInstalled, installed(active = false), installed(active = false))).id,
        )
    }

    @Test
    fun `among multiple active, priority order decides`() {
        // both kmod and zygisk active -> kmod (higher priority).
        val states = states(installed(active = true), ModuleState.NotInstalled, installed(active = true))
        assertEquals(
            NativeBackendId.Kmod,
            states.activeId,
        )
        assertEquals(
            NativeBackendId.Kmod,
            displayNativeBackend(states).id,
        )
    }

    @Test
    fun `detect native backend states wires section map to all detectors`() {
        val states =
            detectNativeBackendStates(
                sections =
                    mapOf(
                        "kmod_prop" to "version=v1.2.3\ngkiVariant=android14-6.1\n",
                        "proc_exists" to "1",
                        "kmod_targets" to "$APP_PACKAGE_NAME\ncom.native.one\n",
                        "kpm_prop" to "version=v2.0.0\n",
                        "kpm_load_status" to "loaded=1\nboot_id=boot-1\n",
                        "kpm_targets" to "$APP_PACKAGE_NAME\ncom.native.two\ncom.native.three\n",
                        "zygisk_prop" to "version=v3.0.0\n",
                        "zygisk_status" to "boot_id=boot-1\n",
                        "zygisk_targets" to "$APP_PACKAGE_NAME\ncom.native.four\n",
                        "current_boot_id" to "boot-1",
                    ),
            )

        assertEquals(
            ModuleState.Installed(
                version = "1.2.3",
                active = true,
                targetCount = 1,
                gkiVariant = "android14-6.1",
            ),
            states.kmod,
        )
        assertEquals(ModuleState.Installed(version = "2.0.0", active = true, targetCount = 2), states.kpm)
        assertEquals(ModuleState.Installed(version = "3.0.0", active = true, targetCount = 1), states.zygisk)
        assertEquals(NativeBackendId.Kmod, states.activeId)
    }

    // ── anyInstalled / noneInstalled ─────────────────────────────────────

    @Test
    fun `none installed is the only state where noneInstalled is true`() {
        val none = states(ModuleState.NotInstalled, ModuleState.NotInstalled, ModuleState.NotInstalled)
        assertEquals(false, none.anyInstalled)
        assertEquals(true, none.noneInstalled)
    }

    @Test
    fun `a single installed backend flips anyInstalled regardless of which one`() {
        val kpmOnly = states(ModuleState.NotInstalled, installed(active = false), ModuleState.NotInstalled)
        assertEquals(true, kpmOnly.anyInstalled)
        assertEquals(false, kpmOnly.noneInstalled)
    }

    // ── classifyMultiNative ──────────────────────────────────────────────

    @Test
    fun `zero or one native active is not an issue`() {
        assertEquals(MultiNativeSeverity.None, classifyMultiNative(false, false, false))
        assertEquals(MultiNativeSeverity.None, classifyMultiNative(true, false, false))
        assertEquals(MultiNativeSeverity.None, classifyMultiNative(false, true, false))
    }

    @Test
    fun `active ko plus active kpm is an error (freeze pair)`() {
        assertEquals(MultiNativeSeverity.Error, classifyMultiNative(kmodActive = true, kpmActive = true, zygiskActive = false))
        // all three active -> still Error because the kmod+kpm pair is present.
        assertEquals(MultiNativeSeverity.Error, classifyMultiNative(kmodActive = true, kpmActive = true, zygiskActive = true))
    }

    @Test
    fun `inactive ko plus active kpm is not an issue`() {
        assertEquals(
            MultiNativeSeverity.None,
            classifyMultiNative(
                kmodActive = moduleActive(installed(active = false)),
                kpmActive = moduleActive(installed(active = true)),
                zygiskActive = false,
            ),
        )
    }

    @Test
    fun `other multi-native active combos are warnings`() {
        assertEquals(MultiNativeSeverity.Warning, classifyMultiNative(kmodActive = true, kpmActive = false, zygiskActive = true))
        assertEquals(MultiNativeSeverity.Warning, classifyMultiNative(kmodActive = false, kpmActive = true, zygiskActive = true))
    }

    // ── kpmDeferredForConflict ───────────────────────────────────────────

    @Test
    fun `kpm deferred-conflict detected for current boot`() {
        val status = "runtime=conflict\nloaded=0\nboot_id=boot-1\ndetail=vpnhide_kmod present\n"
        assertEquals(true, kpmDeferredForConflict(status, currentBootId = "boot-1"))
    }

    @Test
    fun `kpm deferred-conflict ignored for a stale boot`() {
        val status = "runtime=conflict\nloaded=0\nboot_id=boot-0\n"
        assertEquals(false, kpmDeferredForConflict(status, currentBootId = "boot-1"))
    }

    @Test
    fun `kpm deferred-conflict false for non-conflict runtimes and empty status`() {
        assertEquals(false, kpmDeferredForConflict("runtime=activator\nloaded=1\nboot_id=boot-1\n", "boot-1"))
        assertEquals(false, kpmDeferredForConflict("runtime=conflict\nloaded=0\n", "boot-1"))
        assertEquals(false, kpmDeferredForConflict("", "boot-1"))
    }

    // ── kpmAwaitingSuperkey ──────────────────────────────────────────────

    @Test
    fun `kpm awaiting-superkey detected for current boot`() {
        val status = "runtime=apatch\nloaded=0\nboot_id=boot-1\ndetail=awaiting_superkey\n"
        assertEquals(true, kpmAwaitingSuperkey(status, currentBootId = "boot-1"))
    }

    @Test
    fun `kpm awaiting-superkey ignored for a stale boot`() {
        val status = "runtime=apatch\nloaded=0\nboot_id=boot-0\ndetail=awaiting_superkey\n"
        assertEquals(false, kpmAwaitingSuperkey(status, currentBootId = "boot-1"))
    }

    @Test
    fun `kpm awaiting-superkey false once loaded or for other states`() {
        // Superkey saved and module loaded this boot.
        assertEquals(false, kpmAwaitingSuperkey("runtime=kpatch-next\nloaded=1\nboot_id=boot-1\n", "boot-1"))
        // Conflict deferral is a different status, not awaiting-superkey.
        assertEquals(false, kpmAwaitingSuperkey("runtime=conflict\nloaded=0\nboot_id=boot-1\n", "boot-1"))
        assertEquals(false, kpmAwaitingSuperkey("", "boot-1"))
    }

    // ── detectKpmModule ──────────────────────────────────────────────────

    @Test
    fun `kpm not installed when no module prop`() {
        val state = detectKpmModule(emptyMap(), selfPkg = "self", currentBootId = "boot-1")
        assertEquals(ModuleState.NotInstalled, state)
    }

    @Test
    fun `kpm active when loaded for the current boot`() {
        val sections =
            mapOf(
                "kpm_prop" to "id=vpnhide_kpm\nversion=v1.0\n",
                "kpm_load_status" to "loaded=1\nboot_id=boot-1\n",
                "kpm_targets" to "com.example.a\ncom.example.b\n",
            )
        val state = detectKpmModule(sections, selfPkg = "self", currentBootId = "boot-1") as ModuleState.Installed
        assertEquals(true, state.active)
        assertEquals("1.0", state.version)
        assertEquals(2, state.targetCount)
    }

    @Test
    fun `kpm inactive when load status is from a previous boot`() {
        val sections =
            mapOf(
                "kpm_prop" to "id=vpnhide_kpm\nversion=v1.0\n",
                "kpm_load_status" to "loaded=1\nboot_id=old-boot\n",
            )
        val state = detectKpmModule(sections, selfPkg = "self", currentBootId = "boot-1") as ModuleState.Installed
        assertEquals(false, state.active)
    }

    @Test
    fun `kpm inactive when load status has no boot id`() {
        val sections =
            mapOf(
                "kpm_prop" to "id=vpnhide_kpm\nversion=v1.0\n",
                "kpm_load_status" to "loaded=1\ndetail=configured\n",
            )
        val state = detectKpmModule(sections, selfPkg = "self", currentBootId = "boot-1") as ModuleState.Installed
        assertEquals(false, state.active)
    }
}
