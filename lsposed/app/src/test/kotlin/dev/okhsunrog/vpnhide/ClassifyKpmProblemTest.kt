package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ClassifyKpmProblemTest {
    private fun installed(active: Boolean) = ModuleState.Installed(version = "1.0", active = active, targetCount = 0)

    @Test
    fun `not installed produces no problem`() {
        assertNull(classifyKpmProblem(ModuleState.NotInstalled, "runtime=activator\nloaded=0\nboot_id=boot-1\n", "boot-1"))
    }

    @Test
    fun `active kpm is fine regardless of load status`() {
        assertNull(classifyKpmProblem(installed(active = true), "runtime=activator\nloaded=1\nboot_id=boot-1\n", "boot-1"))
    }

    @Test
    fun `conflict runtime is not diagnosed here — handled by kpmDeferredForConflict`() {
        val status = "runtime=conflict\nloaded=0\nboot_id=boot-1\ndetail=vpnhide_kmod present\n"
        assertNull(classifyKpmProblem(installed(active = false), status, "boot-1"))
    }

    @Test
    fun `apatch awaiting-superkey runtime is not diagnosed here — handled by kpmAwaitingSuperkey`() {
        val status = "runtime=apatch\nloaded=0\nboot_id=boot-1\ndetail=awaiting_superkey\n"
        assertNull(classifyKpmProblem(installed(active = false), status, "boot-1"))
    }

    @Test
    fun `missing activator binary is a named diagnosis with a red card`() {
        val status = "runtime=activator\nloaded=0\nboot_id=boot-1\ndetail=activator missing at /data/adb/modules/vpnhide_kpm/activator\n"
        val kind = classifyKpmProblem(installed(active = false), status, "boot-1")
        assertEquals(KpmProblemKind.ActivatorMissing("/data/adb/modules/vpnhide_kpm/activator"), kind)
        assertEquals(ModuleBrokenReason.KpmActivatorMissing, kind?.reason)
    }

    @Test
    fun `generic activator failure surfaces the raw detail with no card color`() {
        val status = "runtime=activator\nloaded=0\nboot_id=boot-1\ndetail=rc=1 supercall failed: ENOENT\n"
        val kind = classifyKpmProblem(installed(active = false), status, "boot-1")
        assertEquals(KpmProblemKind.LoadFailed("rc=1 supercall failed: ENOENT"), kind)
        assertNull(kind?.reason)
    }

    @Test
    fun `stale boot id is ignored`() {
        val status = "runtime=activator\nloaded=0\nboot_id=boot-0\ndetail=rc=1 boom\n"
        assertNull(classifyKpmProblem(installed(active = false), status, "boot-1"))
    }

    @Test
    fun `missing boot id is ignored`() {
        val status = "runtime=activator\nloaded=0\ndetail=rc=1 boom\n"
        assertNull(classifyKpmProblem(installed(active = false), status, "boot-1"))
    }

    @Test
    fun `empty load status is ignored`() {
        assertNull(classifyKpmProblem(installed(active = false), "", "boot-1"))
    }

    @Test
    fun `inactive with fresh configured status but stale active flag is not diagnosed`() {
        // loaded=1 on runtime=activator means "configured" per service.sh, not a failure.
        val status = "runtime=activator\nloaded=1\nboot_id=boot-1\ndetail=configured\n"
        assertNull(classifyKpmProblem(installed(active = false), status, "boot-1"))
    }
}
