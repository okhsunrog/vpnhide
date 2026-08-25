package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.VpnHideState
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckResult
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.NotMeasuredReason
import dev.okhsunrog.vpnhide.diagnostics.Verdict
import dev.okhsunrog.vpnhide.diagnostics.buildDiagnosticReport
import dev.okhsunrog.vpnhide.diagnostics.javaCheck
import dev.okhsunrog.vpnhide.diagnostics.resolveDiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.token
import dev.okhsunrog.vpnhide.diagnostics.verdict
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The canonical report is the single computed diagnostic snapshot: one build,
 * every consumer a pure render. These tests pin the folding — verdicts, the
 * owned/unowned split, per-check attribution carry-through, and the gate — so no
 * renderer has to re-derive (and risk disagreeing with) any of it.
 */
class DiagnosticReportTest {
    private fun installed(active: Boolean) = ModuleState.Installed(version = "1.0", active = active)

    private fun zygisk(state: ModuleState = installed(active = true)) =
        displayNativeBackend(
            NativeBackendStates(kmod = ModuleState.NotInstalled, kpm = ModuleState.NotInstalled, zygisk = state),
        )

    private fun report(
        gate: DiagnosticGate = DiagnosticGate.ROUTED,
        results: CheckResults?,
        backend: DisplayNativeBackend = zygisk(),
        lsposedActive: Boolean = true,
        complete: Boolean = true,
    ) = buildDiagnosticReport(gate, results, backend, lsposedActive, complete)

    /** Native results in the real NATIVE_CHECKS order, carrying the given per-id
     * outcomes; every other probe is left unmeasured. The builder derives the
     * by-id outcome map from this list — there is no separate map to pass. */
    private fun nativeResults(vararg outcomes: Pair<String, CheckOutcome>): List<CheckResult> {
        val byId = outcomes.toMap()
        return NATIVE_CHECKS.map { spec ->
            CheckResult(
                spec.id,
                detail = "",
                outcome = byId[spec.id] ?: CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth),
            )
        }
    }

    // ── native verdict folds off the owned outcomes ────────────────────────

    @Test
    fun `native verdict is Broken when an owned vector leaks and nothing hid`() {
        val r = report(results = CheckResults(native = nativeResults("ioctl_flags" to CheckOutcome.Leak)))
        assertEquals(Verdict.Broken, r.nativeVerdict)
    }

    @Test
    fun `native verdict is Partial when it hides some but an owned vector leaks`() {
        val native = nativeResults("ioctl_flags" to CheckOutcome.Leak, "getifaddrs" to CheckOutcome.HiddenByBackend)
        assertEquals(Verdict.Partial, report(results = CheckResults(native = native)).nativeVerdict)
    }

    @Test
    fun `native verdict is Ok when nothing owned leaks`() {
        val native = nativeResults("ioctl_flags" to CheckOutcome.HiddenByBackend)
        assertEquals(Verdict.Ok, report(results = CheckResults(native = native)).nativeVerdict)
    }

    // ── unowned leaks are surfaced separately, never against the verdict ────

    @Test
    fun `a not-owned leak under zygisk is unowned, not a verdict leak`() {
        // sys_class_net is covered only by the optional FILESYSTEM_IFACE_PATHS hook,
        // which this report is built without installing → out of scope for the zygisk
        // tile, so the verdict stays Ok and the leak is counted as unowned.
        val r = report(results = CheckResults(native = nativeResults("sys_class_net" to CheckOutcome.Leak)))
        assertEquals(Verdict.Ok, r.nativeVerdict)
        assertEquals(1, r.native.unownedLeaks)
    }

    @Test
    fun `a leaking java-implemented native probe folds into the unowned count`() {
        val results =
            CheckResults(
                native = emptyList(),
                nativeExtra = listOf(javaCheck("NetworkInterface enum", clean = false, detail = "tun0 in list")),
            )
        assertEquals(1, report(results = results).native.unownedLeaks)
    }

    // ── per-check attribution carries through verbatim ─────────────────────

    /** Full native list with the first probe (ioctl_flags) carrying a rich leak +
     * root ground-truth, the rest unmeasured — the shape a real leaking run has. */
    private fun ioctlFlagsLeakNative(): List<CheckResult> =
        NATIVE_CHECKS.map { spec ->
            if (spec.id == "ioctl_flags") {
                CheckResult(
                    name = "ioctl SIOCGIFFLAGS tun0",
                    detail = "tun0 is visible!",
                    outcome = CheckOutcome.Leak,
                    groundTruthDetail = "root: tun0 up",
                )
            } else {
                CheckResult(spec.id, detail = "", outcome = CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth))
            }
        }

    @Test
    fun `native check carries id, outcome, ground truth and owned flag`() {
        val checks = report(results = CheckResults(native = ioctlFlagsLeakNative())).native.checks
        val check = checks.first { it.id == "ioctl_flags" }
        assertEquals("ioctl SIOCGIFFLAGS tun0", check.label) // localized label off the result
        assertEquals(CheckOutcome.Leak, check.outcome)
        assertEquals("root: tun0 up", check.groundTruthDetail)
        assertTrue("ioctl_flags is zygisk-owned via zygisk_ioctl", check.owned)
    }

    @Test
    fun `java check carries its gate-derived outcome`() {
        val results =
            CheckResults(
                native = emptyList(),
                coreJava = listOf(CheckResult("hasTransport(VPN)", detail = "VPN!", outcome = CheckOutcome.Leak)),
            )
        val r = report(results = results)
        val javaCheck = r.java.checks.single()
        assertEquals(CheckOutcome.Leak, javaCheck.outcome)
        assertEquals(Verdict.Broken, r.javaVerdict)
    }

    // ── gate ───────────────────────────────────────────────────────────────

    @Test
    fun `a blocked gate carries no checks and no measured verdict`() {
        val r = report(gate = DiagnosticGate.VPN_OFF, results = null)
        assertEquals(DiagnosticGate.VPN_OFF, r.gate)
        assertTrue(r.native.checks.isEmpty())
        assertTrue(r.java.checks.isEmpty())
    }

    @Test
    fun `a blocked gate exposes no verdict`() {
        // A gated run's active layers carry a placeholder Active(0,0). The
        // gate-checked accessors must return null, so no consumer (dashboard tile,
        // exported VpnHideState.nativeVerdict) can fold a false "ok" off the zeros.
        val r = report(gate = DiagnosticGate.VPN_OFF, results = null)
        assertNull(r.nativeVerdict)
        assertNull(r.javaVerdict)
    }

    @Test
    fun `gate folds the three signals worst-first`() {
        assertEquals(DiagnosticGate.VPN_OFF, resolveDiagnosticGate(vpnActive = false, selfRouted = true, selfNeedsRestart = false))
        assertEquals(DiagnosticGate.NEEDS_RESTART, resolveDiagnosticGate(vpnActive = true, selfRouted = true, selfNeedsRestart = true))
        // needs-restart outranks vpn-off: the app's own hooks aren't applied yet, so a
        // run measures nothing regardless of the VPN — reboot is the actionable step.
        assertEquals(DiagnosticGate.NEEDS_RESTART, resolveDiagnosticGate(vpnActive = false, selfRouted = null, selfNeedsRestart = true))
        assertEquals(
            DiagnosticGate.SELF_NOT_ROUTED,
            resolveDiagnosticGate(vpnActive = true, selfRouted = false, selfNeedsRestart = false),
        )
        assertEquals(DiagnosticGate.ROUTED, resolveDiagnosticGate(vpnActive = true, selfRouted = true, selfNeedsRestart = false))
        // A null self-routed answer (no root) does not block.
        assertEquals(DiagnosticGate.ROUTED, resolveDiagnosticGate(vpnActive = true, selfRouted = null, selfNeedsRestart = false))
    }

    // ── the @Serializable report carries the attribution the old bundle dropped ──

    private fun leakReport() = report(results = CheckResults(native = ioctlFlagsLeakNative()))

    @Test
    fun `the report computes the measured verdict from a routed run`() {
        val r = leakReport()
        assertEquals(DiagnosticGate.ROUTED, r.gate)
        // A leak on an owned vector with nothing hidden = Broken (active yet dead).
        assertEquals(Verdict.Broken, r.nativeVerdict)
        val leak = r.native.checks.first { it.id == "ioctl_flags" }
        assertEquals(CheckOutcome.Leak, leak.outcome)
        assertEquals("root: tun0 up", leak.groundTruthDetail)
    }

    @Test
    fun `the report serializes straight to JSON with its outcome and ground truth`() {
        // No hand-written DTO: the domain report IS the serialized form. @SerialName
        // gives the outcome a compact token discriminator, not a fully-qualified name.
        val json = Json { prettyPrint = true }.encodeToString(leakReport())
        assertTrue("compact outcome discriminator", json.contains("\"leak\""))
        assertTrue("ground truth carried", json.contains("\"root: tun0 up\""))
        assertTrue("check id carried", json.contains("\"ioctl_flags\""))
    }
}
