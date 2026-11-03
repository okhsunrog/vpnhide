package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.toDiagnosticInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.vpnhide_core_ffi.*

class CoreStateBindingContractTest {
    private val implementation =
        CoreNativeImplementation(CoreNativeBackend.BUILTIN, CoreNativeImplementationSource.BUILTIN, null, null, null)
    private val native =
        CoreNativeReadModel(
            boot = null,
            selected = implementation,
            applied = CoreNativeBackend.BUILTIN,
            observed = CoreNativeBackend.BUILTIN,
            displayed = implementation,
            observation = CoreObservationFreshness.CURRENT,
            runtime = CoreNativeRuntimeStatus(CoreNativeBackend.BUILTIN, 1u, 3u, 0u),
            observationError = null,
            attempts = emptyList(),
            delivery = CoreNativeDeliveryView(CoreNativeBackend.BUILTIN, 8uL, 4uL, CorePhaseOutcome.CONFIRMED, "a".repeat(64)),
            requiredHooks = 3u,
            installedHooks = 3u,
            missingHooks = 0u,
            setup = CoreNativeSetupHealth.SETUP_READY,
            filesystem = CoreNativeFilesystemSetup.DISABLED,
            processCoverageComplete = false,
            legacyInstalled = emptyList(),
        )
    private val recovery = CoreRecoveryAccess("held", null, null, null, "running hooks")
    private val run =
        CoreDiagnosticRun(
            1uL,
            7uL,
            "boot",
            "a".repeat(64),
            "b".repeat(64),
            null,
            null,
            false,
            3u,
            emptyList(),
            CoreDiagnosticOutcome.Running,
            null,
            false,
            null,
            null,
        )
    private val assessment =
        CoreDiagnosticAssessment(
            CoreAssessmentPhase.RUNNING,
            emptyList(),
            emptyList(),
            emptyList(),
            emptyList(),
            emptyList(),
            emptyList(),
            emptyList(),
            null,
            null,
        )
    private val publication =
        CorePublication(
            zygiskLifecycle = null,
            zygiskRevocation = null,
            autonomousDiagnostics = run,
            autonomousAssessment = assessment,
            recovery = recovery,
            startupError = null,
            storageFault = "storage held",
            epoch = 7uL,
            revision = 12uL,
            configRevision = 4uL,
            config = null,
            operationsInFlight = listOf(8uL),
            mutationHeld = true,
            backend =
                CoreNativeBackendStatus(
                    runtimeConflict = false,
                    observedFreshness = CoreObservationFreshness.CURRENT,
                    runtimeStatus = native.runtime,
                    observationError = null,
                    selected = CoreNativeBackend.BUILTIN,
                    applied = CoreNativeBackend.BUILTIN,
                    active = CoreNativeBackend.BUILTIN,
                    installed = emptyList(),
                    boot = null,
                ),
            nativeView = native,
            debugLease = true,
            debugLeases = emptyList(),
            operations = emptyList(),
            applyEvidence = listOf(CoreApplyEvidence(CoreConfigPhase.NATIVE, 4uL, 8uL, CorePhaseOutcome.UNKNOWN)),
            heldResources = listOf(CoreConfigPhase.NATIVE),
        )

    @Test
    fun `publication binding preserves native evidence holds and recovery together`() {
        val bound = publication.toState()
        assertEquals(listOf(8L), bound.operationsInFlight)
        assertTrue(bound.mutationHeld)
        assertEquals(setOf(ConfigPhase.Native), bound.heldResources)
        assertEquals(listOf(CoreApplyIdentity(ConfigPhase.Native, 4, 8, PhaseOutcome.Unknown)), bound.applyEvidence)
        assertEquals(recovery, bound.recovery)
        assertEquals(run, bound.autonomousDiagnostics)
        assertEquals(assessment, bound.autonomousAssessment)
        assertEquals("storage held", bound.storageFault)
        assertEquals(1, bound.activeCaptures)
        assertEquals("setup_ready", bound.nativeStatus?.setup)
        assertEquals(3L, bound.nativeStatus?.requiredHooks)
        native.requiredHooks = 0u
        native.delivery?.wireSha256 = null
        assertEquals(3L, bound.nativeStatus?.requiredHooks)
        assertEquals("a".repeat(64), bound.nativeStatus?.delivery?.wireSha256)
    }

    @Test
    fun `bind actor evidence remains immutable and separate from installed hook masks`() {
        val result =
            CoreScopedBindResult(listOf(CoreBindActor.LIBC), listOf(CoreBindActor.SYSCALL), listOf(CoreControlFailure.PROBE_UNAVAILABLE))
        val info = run.copy(binding = result).toDiagnosticInfo()
        result.proven = emptyList()
        result.leaked = emptyList()
        result.controls = emptyList()
        assertEquals(listOf("libc"), info.binding?.provenActors)
        assertEquals(listOf("syscall"), info.binding?.leakedActors)
        assertEquals(listOf("probe_unavailable"), info.binding?.controls)
    }

    @Test
    fun `address export keeps actor vector and control evidence detached`() {
        val check = CoreAddressCheck(CoreAddressActor.SYSCALL, CoreAddressVector.IOCTL_ADDRESS)
        val result = CoreScopedAddressResult(emptyList(), listOf(check), listOf(CoreControlFailure.CLEANUP_UNCONFIRMED))
        val info = run.copy(addresses = result).toDiagnosticInfo()
        check.vector = CoreAddressVector.NETLINK_IPV6
        result.leaked = emptyList()
        assertEquals(listOf("syscall.ioctl_address"), info.addresses?.leakedChecks)
        assertEquals(listOf("cleanup_unconfirmed"), info.addresses?.controls)
    }
}
