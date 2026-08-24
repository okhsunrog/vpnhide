package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ClassifyKpmProblemTest {
    private fun installed(active: Boolean) = ModuleState.Installed(version = "1.0", active = active)

    private fun classify(
        kpm: ModuleState,
        rawStatus: String,
        currentBootId: String,
        hasKpatchRuntime: Boolean = true,
        apatchSuperkeySaved: Boolean = true,
    ): KpmProblemKind? =
        classifyKpmProblem(
            kpm = kpm,
            status = parseKpmLoadStatus(rawStatus),
            currentBootId = currentBootId,
            hasKpatchRuntime = hasKpatchRuntime,
            apatchSuperkeySaved = apatchSuperkeySaved,
        )

    @Test
    fun `not installed produces no problem`() {
        assertNull(classify(ModuleState.NotInstalled, "runtime=activator\nloaded=0\nboot_id=boot-1\n", "boot-1"))
    }

    @Test
    fun `active kpm is fine regardless of load status`() {
        assertNull(classify(installed(active = true), "runtime=activator\nloaded=1\nboot_id=boot-1\n", "boot-1"))
    }

    @Test
    fun `conflict runtime is not diagnosed here — handled by kpmDeferredForConflict`() {
        val status = "runtime=conflict\nloaded=0\nboot_id=boot-1\ndetail=vpnhide_kmod present\n"
        assertNull(classify(installed(active = false), status, "boot-1"))
    }

    @Test
    fun `apatch awaiting-superkey runtime is not diagnosed here — handled by kpmAwaitingSuperkey`() {
        val status = "runtime=apatch\nloaded=0\nboot_id=boot-1\ndetail=awaiting_superkey\n"
        assertNull(classify(installed(active = false), status, "boot-1"))
    }

    // The device that motivated this: APatch present, hello accepted via the
    // trusted-su grant, KPM management refused, no SuperKey ever entered. The
    // generic branch told the user to reinstall the zip, which fixes nothing.
    @Test
    fun `refused supercall without a saved superkey asks for the superkey`() {
        val status = "runtime=activator\nloaded=0\nboot_id=boot-1\ndetail=kpm list supercall failed with rc=-1\n"
        val kind = classify(installed(active = false), status, "boot-1", apatchSuperkeySaved = false)
        assertEquals(KpmProblemKind.NeedsSuperkey, kind)
    }

    @Test
    fun `refused supercall with a saved superkey stays the generic failure`() {
        val status = "runtime=activator\nloaded=0\nboot_id=boot-1\ndetail=kpm list supercall failed with rc=-1\n"
        val kind = classify(installed(active = false), status, "boot-1", apatchSuperkeySaved = true)
        assertEquals(KpmProblemKind.LoadFailed("kpm list supercall failed with rc=-1"), kind)
    }

    @Test
    fun `a non-supercall failure is never blamed on the superkey`() {
        val status = "runtime=activator\nloaded=0\nboot_id=boot-1\ndetail=vpnhide.kpm not found\n"
        val kind = classify(installed(active = false), status, "boot-1", apatchSuperkeySaved = false)
        assertEquals(KpmProblemKind.LoadFailed("vpnhide.kpm not found"), kind)
    }

    @Test
    fun `generic activator failure surfaces the raw detail with no card color`() {
        val status = "runtime=activator\nloaded=0\nboot_id=boot-1\ndetail=rc=1 supercall failed: ENOENT\n"
        val kind = classify(installed(active = false), status, "boot-1")
        assertEquals(KpmProblemKind.LoadFailed("rc=1 supercall failed: ENOENT"), kind)
        assertNull(kind?.reason)
    }

    @Test
    fun `no KernelPatch runtime is diagnosed as a missing-runtime problem, not a generic failure`() {
        val status = "runtime=activator\nloaded=0\nboot_id=boot-1\nreason=activation_failed\ndetail=kpatch CLI not found\n"
        val kind = classify(installed(active = false), status, "boot-1", hasKpatchRuntime = false)
        assertEquals(KpmProblemKind.NoKernelPatchRuntime, kind)
        assertNull(kind?.reason)
    }

    @Test
    fun `same failure with a runtime present stays a generic load failure`() {
        val status = "runtime=activator\nloaded=0\nboot_id=boot-1\nreason=activation_failed\ndetail=kpatch CLI not found\n"
        val kind = classify(installed(active = false), status, "boot-1", hasKpatchRuntime = true)
        assertEquals(KpmProblemKind.LoadFailed("kpatch CLI not found"), kind)
    }

    @Test
    fun `unsupported kernel is a named diagnosis`() {
        val status =
            "runtime=activator\nloaded=0\nboot_id=boot-1\nuname_r=4.4.302-vendor\n" +
                "reason=unsupported_kernel\ndetail=unsupported kernel 4.4.302-vendor\n"
        val kind = classify(installed(active = false), status, "boot-1")
        assertEquals(KpmProblemKind.UnsupportedKernel("4.4.302-vendor"), kind)
        assertEquals(ModuleBrokenReason.UnsupportedKernel, kind?.reason)
    }

    @Test
    fun `KPatch-Next load failure is diagnosed too`() {
        val status =
            "runtime=kpatch-next\nloaded=0\nboot_id=boot-1\nreason=load_failed\n" +
                "detail=rc=1 kpatch CLI not found\n"
        val kind = classify(installed(active = false), status, "boot-1")
        assertEquals(KpmProblemKind.LoadFailed("rc=1 kpatch CLI not found"), kind)
    }

    @Test
    fun `stale boot id is ignored`() {
        val status = "runtime=activator\nloaded=0\nboot_id=boot-0\ndetail=rc=1 boom\n"
        assertNull(classify(installed(active = false), status, "boot-1"))
    }

    @Test
    fun `missing boot id is ignored`() {
        val status = "runtime=activator\nloaded=0\ndetail=rc=1 boom\n"
        assertNull(classify(installed(active = false), status, "boot-1"))
    }

    @Test
    fun `empty load status is ignored`() {
        assertNull(classify(installed(active = false), "", "boot-1"))
    }

    @Test
    fun `inactive with fresh configured status but stale active flag is not diagnosed`() {
        // loaded=1 on runtime=activator means "configured" per service.sh, not a failure.
        val status = "runtime=activator\nloaded=1\nboot_id=boot-1\ndetail=configured\n"
        assertNull(classify(installed(active = false), status, "boot-1"))
    }
}
