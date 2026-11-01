package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.*
import org.junit.Assert.*
import org.junit.Test

class ExportRecommendationTest {
    private fun report(
        backend: NativeBackendId,
        nativeLeak: Boolean,
        javaLeak: Boolean,
        owned: Boolean = true,
        complete: Boolean = true,
    ): DiagnosticReport {
        fun layer(
            which: CheckLayer,
            leak: Boolean,
        ) = LayerReport(
            which,
            backend,
            LayerStatus.Active(1, if (leak) 1 else 0),
            0,
            listOf(
                DiagnosticCheck(
                    "test",
                    "test",
                    which,
                    if (leak) CheckOutcome.Leak else CheckOutcome.HiddenByBackend,
                    "",
                    null,
                    emptyList(),
                    owned,
                ),
            ),
        )
        return DiagnosticReport(DiagnosticGate.ROUTED, layer(CheckLayer.NATIVE, nativeLeak), layer(CheckLayer.JAVA, javaLeak), complete)
    }

    @Test fun `owned kernel and Java failures select only related attachments`() {
        assertEquals(
            true to false,
            ExportOverrides().resolve(recommendExport(true, report(NativeBackendId.Builtin, true, false), null, false)),
        )
        assertEquals(
            false to true,
            ExportOverrides().resolve(recommendExport(true, report(NativeBackendId.Zygisk, true, true), null, false)),
        )
        assertEquals(true to true, ExportOverrides().resolve(recommendExport(true, report(NativeBackendId.Kpm, true, true), null, false)))
        assertFalse(recommendExport(true, report(NativeBackendId.Kmod, true, false, owned = false), null, false).kernel)
    }

    @Test fun `missing core incomplete and configuration problems do not imply kernel dumps`() {
        assertFalse(recommendExport(false, report(NativeBackendId.Kmod, true, true), null, true).java)
        assertFalse(recommendExport(true, report(NativeBackendId.Kmod, true, true, complete = false), null, false).kernel)
        val conflict = NativeBootOutcome.Conflict(listOf(NativeBackendId.Kmod, NativeBackendId.Kpm), emptyList(), emptyList())
        assertFalse(recommendExport(true, report(NativeBackendId.Kmod, true, false), conflict, false).kernel)
        val permission = NativeBootOutcome.LoadFailed(NativeBackendId.Kmod, 1, "Permission denied", emptyList(), true, true, false)
        assertFalse(recommendExport(true, null, permission, false).kernel)
        assertTrue(recommendExport(true, null, permission.copy(stderr = "unknown symbol"), false).kernel)
        assertFalse(recommendExport(true, null, permission.copy(signatureRejected = true), false).kernel)
        assertTrue(recommendExport(true, null, null, true).java)
    }

    @Test fun `manual choices survive recommendations refreshing and reset or reopening follows recommendations`() {
        val initial = ExportRecommendation(kernel = true)
        val manual = ExportOverrides(kernel = false, java = true)
        assertEquals(false to true, manual.resolve(initial))
        val refreshed = ExportRecommendation(java = false)
        assertEquals(false to true, manual.resolve(refreshed))
        assertEquals(false to false, ExportOverrides().resolve(refreshed))
        assertEquals(true to false, ExportOverrides().resolve(initial))
    }

    @Test fun `stale interrupted unavailable and partial evidence is not used as a measured failure`() {
        val context = MeasurementContext("subject", "config", "routing", "coverage", 0, 1, 0)
        val measurement = DiagnosticMeasurement(1, context, emptyList(), emptyMap(), true, false, 1)
        val presentation =
            DiagnosticPresentation(
                eligibility = DiagnosticEligibility.Eligible,
                routing = RoutingKnowledge.Known(SelfRouting.Routed, 0),
                activeRunId = null,
                activeRunAutomatic = null,
                activeStage = null,
                activeResults = null,
                lastAttempt = null,
                measurement = measurement,
                measurementResults = CheckResults(native = emptyList()),
                applicability = MeasurementApplicability.MatchesLastObservation,
                evidence = null,
                currentSuccess = false,
                probeUnavailable = false,
            )
        assertTrue(exportMeasurementIsCurrent(presentation))
        for (applicability in listOf(
            MeasurementApplicability.Absent,
            MeasurementApplicability.Changed,
            MeasurementApplicability.Unverified,
        )) {
            assertFalse(exportMeasurementIsCurrent(presentation.copy(applicability = applicability)))
        }
        assertFalse(exportMeasurementIsCurrent(presentation.copy(measurement = measurement.copy(completed = false))))
        assertFalse(exportMeasurementIsCurrent(presentation.copy(measurement = measurement.copy(interrupted = true))))
        assertFalse(exportMeasurementIsCurrent(presentation.copy(eligibility = DiagnosticEligibility.SelfExcluded)))
        assertFalse(exportMeasurementIsCurrent(presentation.copy(desiredDeliveryConfirmed = false)))
        assertFalse(exportMeasurementIsCurrent(presentation.copy(probeUnavailable = true)))
        assertFalse(exportMeasurementIsCurrent(presentation.copy(measurementResults = null)))
    }

    @Test fun `disabled or absent layers do not turn expected leaks into binary recommendations`() {
        val active = report(NativeBackendId.Kmod, true, true)
        val inactive =
            active.copy(
                native = active.native.copy(status = LayerStatus.Inactive),
                java = active.java.copy(status = LayerStatus.Absent),
            )
        val recommendation = recommendExport(true, inactive, null, false)
        assertFalse(recommendation.kernel)
        assertFalse(recommendation.java)
    }
}
