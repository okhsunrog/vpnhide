package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.token
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Covers the `(kernel KMI × series)` → (kmod zip, variant, zygisk?)
 * decision table in [buildNativeInstallRecommendation]. Deliberately
 * paranoid about the reported edge cases — a custom kernel whose
 * `uname -r` lacks the `androidNN` tag, the Android 15 ROM over an
 * `android12-5.10` kernel regression, pre-GKI kernels, etc.
 *
 * `deviceAndroidLabel` is only reflected back in `androidVersion`
 * for display; it is never used for KMI matching. The tests pass
 * both plausible ("Android 12") and mismatched ("Android 15")
 * labels where relevant to prove that invariant.
 */
class BuildNativeInstallRecommendationTest {
    // ── 1. Exact KMI × series matches ─────────────────────────

    @Test
    fun `exact match android12-5_10`() {
        val r = buildNativeInstallRecommendation("5.10.149-android12-foo-g123abc", "Android 12")!!
        assertTrue(r.preferKmod)
        assertFalse(r.variantAmbiguous)
        assertEquals("android12-5.10", r.recommendedGkiVariant)
        assertEquals("vpnhide-kmod-android12-5.10.zip", r.recommendedArtifact)
        assertNull(r.alternativeArtifact)
        assertEquals("Android 12", r.kernelBranch)
        assertEquals("Android 12", r.androidVersion)
    }

    @Test
    fun `exact match covers the full shipping matrix`() {
        data class Case(
            val kernel: String,
            val expectedKmi: String,
        )
        listOf(
            Case("5.10.149-android12-g123abc", "android12-5.10"),
            Case("5.10.149-android13-g123abc", "android13-5.10"),
            Case("5.15.60-android13-g123abc", "android13-5.15"),
            Case("5.15.60-android14-g123abc", "android14-5.15"),
            Case("6.1.25-android14-g123abc", "android14-6.1"),
            Case("6.6.10-android15-g123abc", "android15-6.6"),
            Case("6.12.0-android16-g123abc", "android16-6.12"),
        ).forEach { case ->
            val r = buildNativeInstallRecommendation(case.kernel, "Android 14")
            assertEquals("kernel ${case.kernel}", case.expectedKmi, r?.recommendedGkiVariant)
            assertEquals("kernel ${case.kernel}", true, r?.preferKmod)
            assertEquals("kernel ${case.kernel}", false, r?.variantAmbiguous)
        }
    }

    @Test
    fun `Android 15 ROM on android12-5_10 kernel — reporter's scenario`() {
        // Previous regression: the Android-release fallback turned this
        // into (Android 15, 5.10), which isn't in the shipping table,
        // so the app recommended zygisk. With the KMI-only match it
        // correctly lands on android12-5.10.
        val r = buildNativeInstallRecommendation("5.10.253-android12-Glow-v4.7", "Android 15")!!
        assertTrue(r.preferKmod)
        assertEquals("android12-5.10", r.recommendedGkiVariant)
        // androidVersion reflects the device OS, not the KMI — so the
        // UI shows "Your device: Android 15" with the italic KMI note
        // disambiguating the android12 fragment.
        assertEquals("Android 15", r.androidVersion)
        assertEquals("Android 12", r.kernelBranch)
    }

    // ── 2. Ambiguous fallback (KMI missing, series has 2 shipping variants) ─

    @Test
    fun `no KMI 5_10 returns ambiguous android12 plus android13 alternative`() {
        val r = buildNativeInstallRecommendation("5.10.253-Glow-v4.7", "Android 15")!!
        assertTrue(r.preferKmod)
        assertTrue(r.variantAmbiguous)
        assertEquals("android12-5.10", r.recommendedGkiVariant)
        assertEquals("vpnhide-kmod-android12-5.10.zip", r.recommendedArtifact)
        assertEquals("android13-5.10", r.alternativeGkiVariant)
        assertEquals("vpnhide-kmod-android13-5.10.zip", r.alternativeArtifact)
        assertNull(r.kernelBranch) // no `androidNN` in uname → null
    }

    @Test
    fun `no KMI 5_15 returns ambiguous android13 plus android14 alternative`() {
        val r = buildNativeInstallRecommendation("5.15.104-custom", "Android 14")!!
        assertTrue(r.preferKmod)
        assertTrue(r.variantAmbiguous)
        assertEquals("android13-5.15", r.recommendedGkiVariant)
        assertEquals("android14-5.15", r.alternativeGkiVariant)
    }

    // ── 3. Deterministic fallback (single shipping variant per series) ────

    @Test
    fun `no KMI 6_1 is deterministic not ambiguous`() {
        val r = buildNativeInstallRecommendation("6.1.55-custom-clear", "Android 14")!!
        assertTrue(r.preferKmod)
        assertFalse(r.variantAmbiguous)
        assertEquals("android14-6.1", r.recommendedGkiVariant)
        assertNull(r.alternativeArtifact)
    }

    @Test
    fun `no KMI 6_6 is deterministic`() {
        val r = buildNativeInstallRecommendation("6.6.0-custom", "Android 15")!!
        assertFalse(r.variantAmbiguous)
        assertEquals("android15-6.6", r.recommendedGkiVariant)
        assertNull(r.alternativeGkiVariant)
    }

    @Test
    fun `no KMI 6_12 is deterministic`() {
        val r = buildNativeInstallRecommendation("6.12.1-custom", "Android 16")!!
        assertFalse(r.variantAmbiguous)
        assertEquals("android16-6.12", r.recommendedGkiVariant)
    }

    // ── 4. KMI present but combo not shipping → series fallback ────────────

    @Test
    fun `android14 KMI on 5_10 falls to ambiguous 5_10 — no such shipping combo`() {
        // Android 14 + 5.10 is not a real GKI combo (A14 shipped with
        // 5.15 / 6.1). A kernel carrying that tag is most likely custom;
        // the 5.10 series fallback gives the user both plausible picks.
        val r = buildNativeInstallRecommendation("5.10.100-android14-custom", "Android 14")!!
        assertTrue(r.variantAmbiguous)
        assertEquals("android12-5.10", r.recommendedGkiVariant)
        assertEquals("android13-5.10", r.alternativeGkiVariant)
        // kernelBranch is still parsed — the card can quote "android14" in the note.
        assertEquals("Android 14", r.kernelBranch)
    }

    // ── 5. Non-GKI kernels KernelPatch covers (4.9 / 4.14 / 4.19 / 5.4) → KPM ────

    @Test
    fun `non-GKI 4_9 recommends KPM, not zygisk`() {
        val r = buildNativeInstallRecommendation("4.9.337-perf", "Android 10")!!
        assertEquals(NativeBackendId.Kpm, r.recommended)
        assertFalse(r.preferKmod)
        assertFalse(r.variantAmbiguous)
        assertEquals("vpnhide-kpm.zip", r.recommendedArtifact)
        assertNull(r.recommendedGkiVariant)
    }

    @Test
    fun `non-GKI 4_14 recommends KPM, not zygisk`() {
        // 4.14 has no GKI KMI and no DDK kmod build, but it's in the KPM kver
        // offset table — recommend the universal KPM instead of the
        // detectable zygisk fallback.
        val r = buildNativeInstallRecommendation("4.14.302-g92e0d94b6cba", "Android 13")!!
        assertEquals(NativeBackendId.Kpm, r.recommended)
        assertFalse(r.preferKmod)
        assertFalse(r.variantAmbiguous)
        assertEquals("vpnhide-kpm.zip", r.recommendedArtifact)
        assertNull(r.recommendedGkiVariant)
        assertEquals("Android 13", r.androidVersion)
    }

    @Test
    fun `non-GKI 5_4 recommends KPM`() {
        val r = buildNativeInstallRecommendation("5.4.188-custom", "Android 12")!!
        assertEquals(NativeBackendId.Kpm, r.recommended)
        assertEquals("vpnhide-kpm.zip", r.recommendedArtifact)
    }

    @Test
    fun `KPM recommendation carries the detected runtime flag through`() {
        val withRuntime = buildNativeInstallRecommendation("4.19.157-perf", "Android 11", kpatchRuntimeAvailable = true)!!
        assertEquals(NativeBackendId.Kpm, withRuntime.recommended)
        assertTrue(withRuntime.kpatchRuntimeAvailable)
        val withoutRuntime = buildNativeInstallRecommendation("4.19.157-perf", "Android 11")!!
        assertEquals(NativeBackendId.Kpm, withoutRuntime.recommended)
        assertFalse(withoutRuntime.kpatchRuntimeAvailable)
    }

    // ── 6. Unsupported kernel series → zygisk ─────────────────────────────

    @Test
    fun `4_4 kernel recommends zygisk`() {
        // The KPM kver offset table has no 4.4 entry — fall back to zygisk.
        val r = buildNativeInstallRecommendation("4.4.302-perf", "Android 9")!!
        assertEquals(NativeBackendId.Zygisk, r.recommended)
        assertFalse(r.preferKmod)
        assertEquals("vpnhide-zygisk.zip", r.recommendedArtifact)
        assertNull(r.recommendedGkiVariant)
    }

    @Test
    fun `6_3 recommends zygisk — no shipping variant for this series`() {
        // We only ship kmod for 6.1, 6.6, 6.12 and KPM for 4.9/4.14/4.19/5.4 — a 6.3
        // kernel is rare but shouldn't silently pick one of those.
        val r = buildNativeInstallRecommendation("6.3.0-something", "Android 14")!!
        assertEquals(NativeBackendId.Zygisk, r.recommended)
        assertEquals("vpnhide-zygisk.zip", r.recommendedArtifact)
    }

    @Test
    fun `unparseable kernel string recommends zygisk`() {
        // parseKernelSeries would not find a `\d+\.\d+` token — the
        // function must not crash, just recommend zygisk.
        val r = buildNativeInstallRecommendation("garbage-no-numbers", "Android 12")!!
        assertEquals(NativeBackendId.Zygisk, r.recommended)
        assertEquals("vpnhide-zygisk.zip", r.recommendedArtifact)
    }

    // ── 7. Null / blank input ─────────────────────────────────────────────

    @Test
    fun `empty kernel string returns null`() {
        assertNull(buildNativeInstallRecommendation("", "Android 14"))
    }

    @Test
    fun `blank kernel string returns null`() {
        assertNull(buildNativeInstallRecommendation("   \n  ", "Android 14"))
    }

    // ── 8. Helper parsers ─────────────────────────────────────────────────

    @Test
    fun `parseKernelSeries extracts major minor`() {
        assertEquals("5.10", parseKernelSeries("5.10.253-android12-Glow-v4.7"))
        assertEquals("6.12", parseKernelSeries("6.12.0-something"))
        assertEquals("4.14", parseKernelSeries("4.14.302-g92e0d94b6cba"))
        assertNull(parseKernelSeries("no-version-here"))
    }

    @Test
    fun `parseKernelAndroidBranch finds androidNN anywhere`() {
        assertEquals("Android 12", parseKernelAndroidBranch("5.10.149-android12-Glow"))
        assertEquals("Android 16", parseKernelAndroidBranch("6.12-android16-whatever"))
        assertNull(parseKernelAndroidBranch("5.10.253-Glow-v4.7")) // KMI stripped
        assertNull(parseKernelAndroidBranch("4.14.302-g92e0d94b6cba"))
    }
}
