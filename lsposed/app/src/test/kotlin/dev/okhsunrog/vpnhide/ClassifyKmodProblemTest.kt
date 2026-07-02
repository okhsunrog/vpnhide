package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ClassifyKmodProblemTest {
    private fun installed(
        active: Boolean,
        gkiVariant: String? = null,
    ) = ModuleState.Installed(version = "0.6.3", active = active, targetCount = 0, gkiVariant = gkiVariant)

    private fun loadStatus(
        fresh: Boolean,
        kretprobes: String? = "y",
        unameR: String? = "5.10.0-android12",
        insmodStderr: String? = null,
        insmodExit: Int? = null,
    ) = KmodLoadStatus(
        timestamp = null,
        bootId = "boot",
        unameR = unameR,
        gkiVariant = null,
        kmodVersion = null,
        rootManager = null,
        kprobes = "y",
        kretprobes = kretprobes,
        insmodExit = insmodExit,
        loaded = false,
        insmodStderr = insmodStderr,
        dmesgTail = null,
        freshForCurrentBoot = fresh,
    )

    private fun kmodRecommendation(
        kmi: String,
        artifact: String = "vpnhide-kmod-$kmi.zip",
        ambiguous: Boolean = false,
        altKmi: String? = null,
        altArtifact: String? = null,
        preferKmod: Boolean = true,
    ) = NativeInstallRecommendation(
        androidVersion = "Android 13",
        kernelVersion = "5.10",
        kernelBranch = "Android 13",
        recommended = if (preferKmod) RecommendedBackend.Kmod else RecommendedBackend.Zygisk,
        recommendedArtifact = artifact,
        recommendedGkiVariant = kmi,
        variantAmbiguous = ambiguous,
        alternativeArtifact = altArtifact,
        alternativeGkiVariant = altKmi,
    )

    @Test
    fun `not installed produces no problem`() {
        assertNull(classifyKmodProblem(ModuleState.NotInstalled, kmodRecommendation("android13-5.10"), null))
    }

    @Test
    fun `active kmod with no kprobe issue is fine`() {
        assertNull(classifyKmodProblem(installed(active = true), kmodRecommendation("android13-5.10"), loadStatus(fresh = true)))
    }

    @Test
    fun `missing kretprobes is reported even for an active module`() {
        // The kprobes probe is activity-independent and runs before the
        // active short-circuit.
        val kind =
            classifyKmodProblem(
                installed(active = true),
                kmodRecommendation("android13-5.10"),
                loadStatus(fresh = true, kretprobes = "n"),
            )
        assertEquals(KmodProblemKind.KprobesMissing, kind)
        assertEquals(ModuleBrokenReason.MissingKprobes, kind?.reason)
    }

    @Test
    fun `unsupported kernel falls back to question mark uname without load status`() {
        val rec = kmodRecommendation("", artifact = "vpnhide-zygisk.zip", preferKmod = false)
        val kind = classifyKmodProblem(installed(active = false), rec, loadStatus = null)
        assertEquals(KmodProblemKind.UnsupportedKernel("?", "vpnhide-zygisk.zip"), kind)
    }

    @Test
    fun `unsupported kernel uses load-status uname when available`() {
        val rec = kmodRecommendation("", artifact = "vpnhide-zygisk.zip", preferKmod = false)
        val kind = classifyKmodProblem(installed(active = false), rec, loadStatus(fresh = true, unameR = "4.19.7"))
        assertEquals(KmodProblemKind.UnsupportedKernel("4.19.7", "vpnhide-zygisk.zip"), kind)
    }

    @Test
    fun `wrong stamped variant is a concrete mismatch`() {
        val rec = kmodRecommendation("android13-5.10")
        val kind = classifyKmodProblem(installed(active = false, gkiVariant = "android12-5.10"), rec, null)
        assertEquals(
            KmodProblemKind.WrongVariant("android12-5.10", "android13-5.10", "vpnhide-kmod-android13-5.10.zip"),
            kind,
        )
    }

    @Test
    fun `unknown variant for a kmod-capable kernel`() {
        val rec = kmodRecommendation("android13-5.10")
        val kind = classifyKmodProblem(installed(active = false, gkiVariant = null), rec, null)
        assertEquals(KmodProblemKind.UnknownVariant("vpnhide-kmod-android13-5.10.zip"), kind)
    }

    @Test
    fun `ambiguous series suggests the other candidate`() {
        val rec =
            kmodRecommendation(
                "android12-5.10",
                ambiguous = true,
                altKmi = "android13-5.10",
                altArtifact = "vpnhide-kmod-android13-5.10.zip",
            )
        val kind =
            classifyKmodProblem(
                installed(active = false, gkiVariant = "android12-5.10"),
                rec,
                loadStatus(fresh = true),
            )
        assertEquals(
            KmodProblemKind.AmbiguousLoadFailed("android12-5.10", "vpnhide-kmod-android13-5.10.zip"),
            kind,
        )
    }

    @Test
    fun `installed alternative of an ambiguous series is not a wrong-variant mismatch`() {
        // installed == alternative, so WrongVariant must not fire; it falls
        // through to AmbiguousLoadFailed and suggests the primary.
        val rec =
            kmodRecommendation(
                "android12-5.10",
                ambiguous = true,
                altKmi = "android13-5.10",
                altArtifact = "vpnhide-kmod-android13-5.10.zip",
            )
        val kind =
            classifyKmodProblem(
                installed(active = false, gkiVariant = "android13-5.10"),
                rec,
                loadStatus(fresh = true),
            )
        assertEquals(
            KmodProblemKind.AmbiguousLoadFailed("android13-5.10", "vpnhide-kmod-android12-5.10.zip"),
            kind,
        )
    }

    @Test
    fun `generic insmod failure surfaces stderr when nothing else matches`() {
        // No recommendation, stamped variant present, fresh load with stderr.
        val kind =
            classifyKmodProblem(
                installed(active = false, gkiVariant = "android13-5.10"),
                recommendation = null,
                loadStatus = loadStatus(fresh = true, insmodStderr = "exec format error"),
            )
        assertEquals(KmodProblemKind.LoadFailed("exec format error"), kind)
        assertNull(kind?.reason)
    }

    @Test
    fun `inactive with no diagnosable cause is null`() {
        // No recommendation, no fresh load status: nothing to say.
        assertNull(classifyKmodProblem(installed(active = false, gkiVariant = "android13-5.10"), null, null))
    }

    @Test
    fun `EKEYREJECTED exit code is diagnosed as signature enforcement`() {
        val kind =
            classifyKmodProblem(
                installed(active = false, gkiVariant = "android13-5.10"),
                kmodRecommendation("android13-5.10"),
                loadStatus(fresh = true, insmodStderr = "Key was rejected by service", insmodExit = 129),
            )
        assertEquals(KmodProblemKind.SignatureEnforced, kind)
        assertEquals(ModuleBrokenReason.SignatureEnforced, kind?.reason)
    }

    @Test
    fun `signature rejection is matched from stderr when exit code is unavailable`() {
        val kind =
            classifyKmodProblem(
                installed(active = false, gkiVariant = "android13-5.10"),
                kmodRecommendation("android13-5.10"),
                loadStatus(fresh = true, insmodStderr = "init_module: Key was rejected by service"),
            )
        assertEquals(KmodProblemKind.SignatureEnforced, kind)
    }

    @Test
    fun `signature enforcement outranks a wrong-variant diagnosis`() {
        // An enforcing kernel rejects every unsigned .ko before vermagic is
        // even checked, so EKEYREJECTED must not be misreported as a fixable
        // variant mismatch.
        val kind =
            classifyKmodProblem(
                installed(active = false, gkiVariant = "android12-5.10"),
                kmodRecommendation("android13-5.10"),
                loadStatus(fresh = true, insmodStderr = "Key was rejected by service", insmodExit = 129),
            )
        assertEquals(KmodProblemKind.SignatureEnforced, kind)
    }

    @Test
    fun `unrelated insmod failure is not treated as signature enforcement`() {
        val kind =
            classifyKmodProblem(
                installed(active = false, gkiVariant = "android13-5.10"),
                recommendation = null,
                loadStatus = loadStatus(fresh = true, insmodStderr = "exec format error", insmodExit = 8),
            )
        assertEquals(KmodProblemKind.LoadFailed("exec format error"), kind)
    }
}
