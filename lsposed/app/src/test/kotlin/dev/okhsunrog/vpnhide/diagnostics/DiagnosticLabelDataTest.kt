package dev.okhsunrog.vpnhide.diagnostics

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DiagnosticLabelDataTest {
    @Test
    fun retainedMeasurementsCanResolveEveryCurrentProbeLabel() {
        NATIVE_CHECKS.forEach { assertEquals(it.labelRes, diagnosticLabelResource(it.id)) }
        (NATIVE_EXTRA_CHECKS + CORE_JAVA_CHECKS + EXTRA_JAVA_CHECKS).forEach {
            assertEquals(it.labelRes, diagnosticLabelResource(it.id))
        }
    }

    @Test
    fun unknownProbeKeepsItsRecordedLabel() {
        assertNull(diagnosticLabelResource("future-probe"))
    }
}
