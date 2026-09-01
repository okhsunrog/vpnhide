package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.standaloneKpmLoaded
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NativeBackendTest {
    private fun installed(active: Boolean) = ModuleState.Installed(version = "1.0", active = active)

    private fun parseStatus(raw: String): KpmLoadStatus = parseKpmLoadStatus(raw)

    private fun detectKpm(
        sections: Map<String, String>,
        currentBootId: String,
    ): ModuleState = detectKpmModule(sections, parseStatus(sections["kpm_load_status"].orEmpty()), currentBootId)

    private fun states(
        kmod: ModuleState,
        kpm: ModuleState,
        zygisk: ModuleState,
        builtin: ModuleState = ModuleState.NotInstalled,
    ) = NativeBackendStates(kmod = kmod, kpm = kpm, zygisk = zygisk, builtin = builtin)

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
    fun `builtin sits between kmod and kpm in display priority`() {
        // Builtin installed alongside KPM/Zygisk, none active -> builtin shown
        // (it is a top-tier kernel backend, directly below the .ko).
        assertEquals(
            NativeBackendId.Builtin,
            displayNativeBackend(
                states(
                    kmod = ModuleState.NotInstalled,
                    kpm = installed(active = false),
                    zygisk = installed(active = false),
                    builtin = installed(active = false),
                ),
            ).id,
        )
        // An active builtin is reported over an inactive higher-priority kmod.
        val builtinActive =
            states(
                kmod = installed(active = false),
                kpm = ModuleState.NotInstalled,
                zygisk = ModuleState.NotInstalled,
                builtin = installed(active = true),
            )
        assertEquals(NativeBackendId.Builtin, builtinActive.activeId)
        assertEquals(true, builtinActive.anyInstalled)
    }

    @Test
    fun `detect native backend states resolves builtin from a backend 0x4 control reply`() {
        val states =
            detectNativeBackendStates(
                sections =
                    mapOf(
                        "kmod_prop" to "id=vpnhide_kmod\nversion=v1.2.5\n",
                        "builtin_prop" to "id=vpnhide_builtin\nversion=v1.2.5\n",
                        "proc_exists" to "1",
                        "kmod_state" to "vpnhide 1 status\nbackend 0x4\nkver 0x0\nhooks 0x3ff\nerror 0x0\n",
                        "current_boot_id" to "boot-1",
                    ),
            )
        // Built-in owns the live proc; the .ko is installed but not the thing running.
        assertEquals(NativeBackendId.Builtin, states.activeId)
        assertEquals(true, moduleActive(states.builtin))
        assertEquals(false, moduleActive(states.kmod))
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
                        "kpm_prop" to "version=v2.0.0\n",
                        "kpm_load_status" to "loaded=1\nboot_id=boot-1\n",
                        "zygisk_prop" to "version=v3.0.0\n",
                        "zygisk_status" to "boot_id=boot-1\n",
                        "current_boot_id" to "boot-1",
                    ),
            )

        assertEquals(
            ModuleState.Installed(
                version = "1.2.3",
                active = true,
                gkiVariant = "android14-6.1",
            ),
            states.kmod,
        )
        assertEquals(ModuleState.Installed(version = "2.0.0", active = true), states.kpm)
        assertEquals(ModuleState.Installed(version = "3.0.0", active = true), states.zygisk)
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
        assertEquals(true, kpmDeferredForConflict(parseStatus(status), currentBootId = "boot-1"))
    }

    @Test
    fun `kpm deferred-conflict ignored for a stale boot`() {
        val status = "runtime=conflict\nloaded=0\nboot_id=boot-0\n"
        assertEquals(false, kpmDeferredForConflict(parseStatus(status), currentBootId = "boot-1"))
    }

    @Test
    fun `kpm deferred-conflict false for non-conflict runtimes and empty status`() {
        assertEquals(false, kpmDeferredForConflict(parseStatus("runtime=activator\nloaded=1\nboot_id=boot-1\n"), "boot-1"))
        assertEquals(false, kpmDeferredForConflict(parseStatus("runtime=conflict\nloaded=0\n"), "boot-1"))
        assertEquals(false, kpmDeferredForConflict(parseStatus(""), "boot-1"))
    }

    // ── kpmAwaitingSuperkey ──────────────────────────────────────────────

    @Test
    fun `kpm awaiting-superkey detected for current boot`() {
        val status = "runtime=apatch\nloaded=0\nboot_id=boot-1\nreason=awaiting_superkey\ndetail=awaiting_superkey\n"
        assertEquals(true, kpmAwaitingSuperkey(parseStatus(status), currentBootId = "boot-1"))
    }

    @Test
    fun `kpm awaiting-superkey ignored for a stale boot`() {
        val status = "runtime=apatch\nloaded=0\nboot_id=boot-0\nreason=awaiting_superkey\ndetail=awaiting_superkey\n"
        assertEquals(false, kpmAwaitingSuperkey(parseStatus(status), currentBootId = "boot-1"))
    }

    @Test
    fun `kpm awaiting-superkey false once loaded or for other states`() {
        // Superkey saved and module loaded this boot.
        assertEquals(false, kpmAwaitingSuperkey(parseStatus("runtime=kpatch-next\nloaded=1\nboot_id=boot-1\n"), "boot-1"))
        // Conflict deferral is a different status, not awaiting-superkey.
        assertEquals(false, kpmAwaitingSuperkey(parseStatus("runtime=conflict\nloaded=0\nboot_id=boot-1\n"), "boot-1"))
        assertEquals(false, kpmAwaitingSuperkey(parseStatus(""), "boot-1"))
    }

    // ── kpatchRuntimeAvailable ───────────────────────────────────────────

    @Test
    fun `kpatch runtime available when APatch or FolkPatch directory is present`() {
        val runtime =
            """
            apatch_dir=1
            superkey_saved=0
            kpatch_bin=(not found)
            """.trimIndent()
        assertEquals(true, kpatchRuntimeAvailable(runtime))
    }

    @Test
    fun `kpatch runtime available when KPatch-Next hello succeeds`() {
        val runtime =
            """
            apatch_dir=0
            superkey_saved=0
            kpatch_bin=/data/adb/modules/KPatch-Next/bin/kpatch
            hello_exit=0
            list_exit=0
            """.trimIndent()
        assertEquals(true, kpatchRuntimeAvailable(runtime))
    }

    @Test
    fun `kpatch runtime unavailable when neither APatch nor KPatch-Next responds`() {
        val runtime =
            """
            apatch_dir=0
            superkey_saved=0
            kpatch_bin=(not found)
            """.trimIndent()
        assertEquals(false, kpatchRuntimeAvailable(runtime))
    }

    @Test
    fun `kpatch runtime unavailable when KPatch-Next is installed but the kernel is not patched`() {
        // Binary present but `kpatch hello` failed — the module is installed yet
        // the boot image was never patched from its UI, so no live runtime.
        val runtime =
            """
            apatch_dir=0
            kpatch_bin=/data/adb/modules/KPatch-Next/bin/kpatch
            hello_exit=1
            """.trimIndent()
        assertEquals(false, kpatchRuntimeAvailable(runtime))
    }

    // ── detectKpmModule ──────────────────────────────────────────────────

    @Test
    fun `kpm not installed when no module prop`() {
        val state = detectKpm(emptyMap(), currentBootId = "boot-1")
        assertEquals(ModuleState.NotInstalled, state)
    }

    @Test
    fun `kpm active when loaded for the current boot`() {
        val sections =
            mapOf(
                "kpm_prop" to "id=vpnhide_kpm\nversion=v1.0\n",
                "kpm_load_status" to "loaded=1\nboot_id=boot-1\n",
            )
        val state = detectKpm(sections, currentBootId = "boot-1") as ModuleState.Installed
        assertEquals(true, state.active)
        assertEquals("1.0", state.version)
    }

    @Test
    fun `kpm inactive when load status is from a previous boot`() {
        val sections =
            mapOf(
                "kpm_prop" to "id=vpnhide_kpm\nversion=v1.0\n",
                "kpm_load_status" to "loaded=1\nboot_id=old-boot\n",
            )
        val state = detectKpm(sections, currentBootId = "boot-1") as ModuleState.Installed
        assertEquals(false, state.active)
    }

    @Test
    fun `kpm inactive when load status has no boot id`() {
        val sections =
            mapOf(
                "kpm_prop" to "id=vpnhide_kpm\nversion=v1.0\n",
                "kpm_load_status" to "loaded=1\ndetail=configured\n",
            )
        val state = detectKpm(sections, currentBootId = "boot-1") as ModuleState.Installed
        assertEquals(false, state.active)
    }

    @Test
    fun `standalone runtime KPM is detected without flashable module`() {
        assertTrue(standaloneKpmLoaded(ModuleState.NotInstalled, "available=1\nvpnhide\n"))
        assertTrue(standaloneKpmLoaded(ModuleState.NotInstalled, "available=1\n0 vpnhide loaded\n"))
    }

    @Test
    fun `runtime KPM is not standalone when flashable module is installed`() {
        val installed = ModuleState.Installed(version = "1.0", active = true)
        assertFalse(standaloneKpmLoaded(installed, "available=1\nvpnhide\n"))
    }

    @Test
    fun `unavailable runtime list or another KPM is not a standalone vpnhide install`() {
        assertFalse(standaloneKpmLoaded(ModuleState.NotInstalled, "available=0\n"))
        assertFalse(standaloneKpmLoaded(ModuleState.NotInstalled, "available=1\nother_module\n"))
        assertFalse(standaloneKpmLoaded(ModuleState.NotInstalled, "available=1\nvpnhide_next\n"))
    }
}
