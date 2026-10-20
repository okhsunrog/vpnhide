package dev.okhsunrog.vpnhide

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The app half of core's `NativeBootStatus` wire. The fixture is serialized by core-rpc's
 * `native_boot_status_matches_app_fixture`, so a wire change on either side fails a test.
 */
class NativeBootStatusTest {
    private val fixture: JSONArray =
        JSONArray(File("../../fixtures/core-rpc/native-boot-status.json").readText())

    private fun entry(index: Int): NativeBootStatus = requireNotNull(parseNativeBootStatus(fixture.getJSONObject(index).toString()))

    @Test
    fun `every core outcome in the fixture parses`() {
        val statuses = List(fixture.length()) { entry(it) }
        val kinds = statuses.map { it.outcome::class }.toSet()

        assertEquals(16, statuses.size)
        assertEquals(13, kinds.size)
        assertEquals(NativeBootOutcome.Selected(NativeBackendId.Kmod, NativeSelectionMode.Loaded, true), statuses[0].outcome)
        assertEquals(NativeBootOutcome.Selected(NativeBackendId.Builtin, NativeSelectionMode.Adopted, null), statuses[1].outcome)
        val first = statuses[0]
        assertEquals("b-1", first.bootId)
        assertEquals(listOf(NativeBackendId.Kmod), first.installed)
        assertEquals(listOf(NativeBackendId.Kpm), first.live)
        assertEquals(listOf(NativeBackendId.Zygisk), first.staged)
        assertEquals(KmodBootReport(true, false, "android14-6.1", "2.0.0", "6.1.0"), first.kmod)
        assertEquals(ZygiskTargetsState.Retired, first.zygiskTargets)
        assertEquals(listOf("vpnhide_ports"), first.obsoleteModules)
        val ports = requireNotNull(first.ports)
        assertEquals(PortsBootSource.BootRetry, ports.source)
        assertEquals(FamilyOutcome.Unknown, ports.ipv6)
        assertEquals(false, ports.netdReady)
        assertFalse(ports.applied)
        val last = statuses.last()
        assertEquals(ZygiskTargetsState.RetireFailed("read-only"), last.zygiskTargets)
        assertNull(last.kmod)
        assertNull(last.ports)
        assertEquals(NativeBootOutcome.NoBackend, last.outcome)
    }

    @Test
    fun `an incompatible live KPM keeps the reported and required versions`() {
        val incompatible = List(fixture.length()) { entry(it).outcome }.filterIsInstance<NativeBootOutcome.KpmIncompatible>()

        assertEquals(
            listOf(
                NativeBootOutcome.KpmIncompatible("1.1.1", "1.2.0", "too old"),
                NativeBootOutcome.KpmIncompatible(null, "1.2.0", "no status"),
            ),
            incompatible,
        )
    }

    @Test
    fun `a capacity refusal keeps what to deselect`() {
        val refused = List(fixture.length()) { entry(it).outcome }.filterIsInstance<NativeBootOutcome.CapacityExceeded>().single()

        assertEquals(NativeBackendId.Kmod, refused.backend)
        assertEquals(
            NativeCapacityRefusal(
                total = 162,
                capacity = 160,
                packages =
                    listOf(
                        NativeCapacityPackage("com.cloned", listOf(10_123, 1_010_123)),
                        NativeCapacityPackage("com.example", listOf(10_200)),
                    ),
            ),
            refused.report,
        )
        assertEquals(2, refused.report.overflow)
    }

    @Test
    fun `the load failure keeps the kernel evidence`() {
        val failed = List(fixture.length()) { entry(it).outcome }.filterIsInstance<NativeBootOutcome.LoadFailed>().single()

        assertEquals(NativeBackendId.Kmod, failed.backend)
        assertEquals(1, failed.exit)
        assertEquals(listOf("vpnhide: x"), failed.dmesgTail)
        assertEquals(true, failed.kprobes)
        assertNull(failed.kretprobes)
        assertTrue(failed.signatureRejected)
    }

    @Test
    fun `malformed or newer vocabularies are unknown, never guessed`() {
        val good = fixture.getJSONObject(0)
        assertNull(parseNativeBootStatus(""))
        assertNull(parseNativeBootStatus("$good trailing"))
        assertNull(parseNativeBootStatus(JSONObject(good.toString()).put("installed", JSONArray(listOf("Ebpf"))).toString()))
        assertNull(
            parseNativeBootStatus(JSONObject(good.toString()).put("outcome", JSONObject().put("Rebooting", JSONObject())).toString()),
        )
        assertNull(parseNativeBootStatus(JSONObject(good.toString()).put("outcome", "Selected").toString()))
        assertNull(parseNativeBootStatus(JSONObject(good.toString()).put("zygisk_targets", "Exploded").toString()))
        assertNull(parseNativeBootStatus(JSONObject(good.toString()).apply { remove("boot_id") }.toString()))
    }

    @Test
    fun `the boot decision is lifted out of core's state reply`() {
        val boot = fixture.getJSONObject(0)
        val state = JSONObject().put("backend", JSONObject().put("installed", JSONArray()).put("boot", boot))

        assertNotNull(nativeBootFromCoreState(state)?.let(::parseNativeBootStatus))
        assertNull(nativeBootFromCoreState(JSONObject().put("backend", JSONObject().put("boot", JSONObject.NULL))))
        assertNull(nativeBootFromCoreState(JSONObject()))
        assertNull(nativeBootFromCoreState(JSONObject().put("backend", JSONObject().put("boot", JSONObject().put("boot_id", "x")))))
    }

    @Test
    fun `the snapshot carries the status under its own key`() {
        val raw = fixture.getJSONObject(0).toString()

        assertEquals("b-1", nativeBootStatus(mapOf(NATIVE_BOOT_SECTION to raw))?.bootId)
        assertNull(nativeBootStatus(emptyMap()))
    }

    // ── Adapters ──

    private fun status(
        outcome: NativeBootOutcome,
        kmod: KmodBootReport? = null,
        staged: List<NativeBackendId> = emptyList(),
        installed: List<NativeBackendId> = emptyList(),
    ) = NativeBootStatus("boot-1", 5_000, outcome, installed, emptyList(), staged, kmod, ZygiskTargetsState.NotInstalled, null)

    @Test
    fun `KPM view of each outcome`() {
        val selected = status(NativeBootOutcome.Selected(NativeBackendId.Kpm, NativeSelectionMode.Loaded, true)).kpmLoadStatus()
        assertEquals(true, selected.loaded)
        assertEquals(true, selected.filesystemHiding)
        assertTrue(selected.isFreshFor("boot-1"))

        val awaiting = status(NativeBootOutcome.AwaitingAuthentication("auth")).kpmLoadStatus()
        assertTrue(kpmAwaitingSuperkey(awaiting, "boot-1"))
        assertFalse(kpmAwaitingSuperkey(awaiting, "boot-2"))

        val conflict =
            status(NativeBootOutcome.Conflict(listOf(NativeBackendId.Kmod, NativeBackendId.Kpm), emptyList(), emptyList())).kpmLoadStatus()
        assertEquals(KpmRuntime.Conflict, conflict.runtime)
        assertEquals(KpmFailureReason.ConflictingBackend, conflict.reason)

        val unsupported = status(NativeBootOutcome.UnsupportedKernel(NativeBackendId.Kpm, "6.10.0", "no offsets")).kpmLoadStatus()
        assertEquals(KpmFailureReason.UnsupportedKernel, unsupported.reason)
        assertEquals("6.10.0", unsupported.unameR)

        assertEquals(
            KpmFailureReason.MissingKpm,
            status(NativeBootOutcome.MissingArtifact(NativeBackendId.Kpm, "/x.kpm")).kpmLoadStatus().reason,
        )
        assertEquals(
            KpmFailureReason.ActivationFailed,
            status(NativeBootOutcome.DeliveryFailed(NativeBackendId.Kpm, "ctl")).kpmLoadStatus().reason,
        )
        // An outcome about another backend says nothing about the KPM.
        assertEquals(
            KPM_STATUS_NONE,
            status(NativeBootOutcome.Selected(NativeBackendId.Kmod, NativeSelectionMode.Loaded, null)).kpmLoadStatus(),
        )
        assertEquals(KPM_STATUS_NONE, (null as NativeBootStatus?).kpmLoadStatus())
    }

    @Test
    fun `kmod view feeds the kmod diagnoses`() {
        val kmod = ModuleState.Installed(version = "2.0.0", active = false, gkiVariant = "android14-6.1")
        val rejected =
            status(NativeBootOutcome.LoadFailed(NativeBackendId.Kmod, 1, "Key was rejected by service", listOf("a", "b"), true, true, true))
                .kmodLoadStatus("boot-1")
        assertEquals("a\nb", rejected?.dmesgTail)
        assertEquals(KmodProblemKind.SignatureEnforced, classifyKmodProblem(kmod, null, rejected))

        val noKretprobes =
            status(
                NativeBootOutcome.NoBackend,
                kmod = KmodBootReport(true, false, "android14-6.1", "2.0.0", "6.1.0"),
            ).kmodLoadStatus("boot-1")
        assertEquals(KmodProblemKind.KprobesMissing, classifyKmodProblem(kmod, null, noKretprobes))

        val loaded = status(NativeBootOutcome.Selected(NativeBackendId.Kmod, NativeSelectionMode.Loaded, true)).kmodLoadStatus("boot-1")
        assertEquals(true, loaded?.loaded)
        assertEquals(true, loaded?.filesystemHiding)

        val stale = status(NativeBootOutcome.Selected(NativeBackendId.Kmod, NativeSelectionMode.Loaded, true)).kmodLoadStatus("boot-2")
        assertEquals(false, stale?.freshForCurrentBoot)

        assertNull(status(NativeBootOutcome.Selected(NativeBackendId.Kpm, NativeSelectionMode.Loaded, true)).kmodLoadStatus("boot-1"))
    }

    @Test
    fun `staged conflict counts distinct backends after the reboot`() {
        val selected = NativeBootOutcome.Selected(NativeBackendId.Zygisk, NativeSelectionMode.Adopted, null)
        assertEquals(
            listOf(NativeBackendId.Kmod, NativeBackendId.Zygisk),
            status(selected, staged = listOf(NativeBackendId.Kmod), installed = listOf(NativeBackendId.Zygisk)).stagedConflict,
        )
        assertEquals(
            emptyList<NativeBackendId>(),
            status(selected, staged = emptyList(), installed = listOf(NativeBackendId.Zygisk)).stagedConflict,
        )
        assertEquals(emptyList<NativeBackendId>(), status(NativeBootOutcome.NoBackend, staged = listOf(NativeBackendId.Kpm)).stagedConflict)
    }
}
