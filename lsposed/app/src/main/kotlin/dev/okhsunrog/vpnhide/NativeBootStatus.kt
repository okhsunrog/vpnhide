package dev.okhsunrog.vpnhide

import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener

/**
 * Core's once-per-boot native decision (`core-rpc` `NativeBootStatus`, protocol 7), read from
 * core's published state over the app socket. Core is the only activator: it scans the
 * installed and live native backends once per boot, loads or adopts exactly one, and refuses
 * to load anything when more than one is a candidate. This replaces the per-backend
 * `load_status` files the module boot scripts used to write.
 */
internal data class NativeBootStatus(
    val bootId: String,
    val decidedAtMs: Long,
    val outcome: NativeBootOutcome,
    /** Installed native modules (module.prop, not disabled, not scheduled for removal). */
    val installed: List<NativeBackendId>,
    /** Live runtime identities at the scan (built-in kernel, a live kmod or KPM). */
    val live: List<NativeBackendId>,
    /** Modules staged only in `modules_update`: candidates after the next reboot. */
    val staged: List<NativeBackendId>,
    val kmod: KmodBootReport?,
    val zygiskTargets: ZygiskTargetsState,
    val ports: PortsBootStatus?,
) {
    fun isFreshFor(currentBootId: String): Boolean = bootId.isNotEmpty() && bootId == currentBootId.trim()

    /**
     * The native backends that will be candidates after the next reboot, when a staged
     * module would make that more than one. Empty when there is nothing to warn about, or
     * when this boot is already a [NativeBootOutcome.Conflict] (that banner covers it).
     */
    val stagedConflict: List<NativeBackendId>
        get() {
            if (staged.isEmpty() || outcome is NativeBootOutcome.Conflict) return emptyList()
            val next = (installed + live + staged).distinct().sortedBy { it.ordinal }
            return if (next.size > 1) next else emptyList()
        }
}

internal enum class NativeSelectionMode { Adopted, Loaded }

internal sealed interface NativeBootOutcome {
    data class Selected(
        val backend: NativeBackendId,
        val mode: NativeSelectionMode,
        val filesystemHiding: Boolean?,
    ) : NativeBootOutcome

    data class Conflict(
        val installed: List<NativeBackendId>,
        val live: List<NativeBackendId>,
        val staged: List<NativeBackendId>,
    ) : NativeBootOutcome {
        /** Every backend that took part in the conflict, in a stable order. */
        val backends: List<NativeBackendId> get() = (installed + live).distinct().sortedBy { it.ordinal }
    }

    data class AwaitingAuthentication(
        val detail: String,
    ) : NativeBootOutcome

    data class KpmUnverifiable(
        val detail: String,
    ) : NativeBootOutcome

    /**
     * A VPN Hide KPM is live — loaded by core earlier, autoloaded by APatch or loaded by
     * hand — but core cannot drive it ([found] is too old / unrecognised, or it failed the
     * status handshake). Nothing was configured and no second copy was loaded.
     */
    data class KpmIncompatible(
        val found: String?,
        val minimum: String,
        val detail: String,
    ) : NativeBootOutcome

    /**
     * The saved config projects to more target UIDs than the selected backend holds (a
     * Repair or boot delivery). Core sent nothing: the backend keeps its previous targets.
     */
    data class CapacityExceeded(
        val backend: NativeBackendId,
        val report: NativeCapacityRefusal,
    ) : NativeBootOutcome

    data class UnsupportedKernel(
        val backend: NativeBackendId,
        val uname: String,
        val detail: String,
    ) : NativeBootOutcome

    data class MissingArtifact(
        val backend: NativeBackendId,
        val path: String,
    ) : NativeBootOutcome

    data class LoadFailed(
        val backend: NativeBackendId,
        val exit: Int?,
        val stderr: String,
        val dmesgTail: List<String>,
        val kprobes: Boolean?,
        val kretprobes: Boolean?,
        val signatureRejected: Boolean,
    ) : NativeBootOutcome

    data class DeliveryFailed(
        val backend: NativeBackendId,
        val detail: String,
    ) : NativeBootOutcome

    data class ForeignOwner(
        val id: Long,
    ) : NativeBootOutcome

    data object NoBackend : NativeBootOutcome

    data class Unknown(
        val detail: String,
    ) : NativeBootOutcome
}

internal data class KmodBootReport(
    val kprobes: Boolean?,
    val kretprobes: Boolean?,
    val gkiVariant: String,
    val version: String,
    val uname: String,
)

internal sealed interface ZygiskTargetsState {
    data object NotInstalled : ZygiskTargetsState

    data object Pending : ZygiskTargetsState

    data object Published : ZygiskTargetsState

    data object Retired : ZygiskTargetsState

    data class RetireFailed(
        val detail: String,
    ) : ZygiskTargetsState
}

internal enum class PortsBootSource { Boot, BootRetry, Edit }

internal enum class FamilyOutcome { Confirmed, FailedKnown, Unknown, NotAttempted }

internal data class PortsBootStatus(
    val source: PortsBootSource,
    val ipv4: FamilyOutcome,
    val ipv6: FamilyOutcome,
    val targetCount: Int,
    val netdReady: Boolean?,
    val detail: String,
    val atMs: Long,
) {
    /** Both families applied (or a family was not needed). */
    val applied: Boolean
        get() = listOf(ipv4, ipv6).all { it == FamilyOutcome.Confirmed || it == FamilyOutcome.NotAttempted }
}

/**
 * The root-snapshot key the loader stores core's native status JSON under. Not a probe
 * section: the value comes from core's published state over RPC, not from a file.
 */
internal const val NATIVE_BOOT_SECTION = "core_native_boot"

/** The typed native status carried in a root snapshot, or null when core did not report one. */
internal fun nativeBootStatus(sections: Map<String, String>): NativeBootStatus? =
    parseNativeBootStatus(sections[NATIVE_BOOT_SECTION].orEmpty())

/**
 * Strict parse of the serde JSON of `NativeBootStatus` (externally tagged enums). Anything
 * malformed or from a newer vocabulary is null: unknown is never guessed into a state.
 */
internal fun parseNativeBootStatus(raw: String): NativeBootStatus? =
    try {
        if (raw.isBlank()) {
            null
        } else {
            val tokener = JSONTokener(raw)
            val json = tokener.nextValue() as JSONObject
            require(tokener.nextClean() == '\u0000')
            nativeBootStatusFrom(json)
        }
    } catch (_: Exception) {
        null
    }

/** The `backend.boot` object of a core `state` reply, or null when absent or malformed. */
internal fun nativeBootFromCoreState(state: JSONObject): String? {
    val backend = state.optJSONObject("backend") ?: return null
    val boot = backend.optJSONObject("boot") ?: return null
    return boot.toString().takeIf { runCatching { nativeBootStatusFrom(boot) }.isSuccess }
}

private fun nativeBootStatusFrom(json: JSONObject): NativeBootStatus =
    NativeBootStatus(
        bootId = json.getString("boot_id"),
        decidedAtMs = json.getLong("decided_at_ms"),
        outcome = outcome(json.get("outcome")),
        installed = backends(json.getJSONArray("installed")),
        live = backends(json.getJSONArray("live")),
        staged = backends(json.getJSONArray("staged")),
        kmod = json.optNullable("kmod")?.let { kmodReport(it as JSONObject) },
        zygiskTargets = zygiskTargets(json.get("zygisk_targets")),
        ports = json.optNullable("ports")?.let { ports(it as JSONObject) },
    )

private fun JSONObject.optNullable(key: String): Any? = if (!has(key) || isNull(key)) null else get(key)

private fun JSONObject.optBool(key: String): Boolean? = optNullable(key)?.let { it as Boolean }

private fun backend(raw: String): NativeBackendId = NativeBackendId.entries.first { it.name == raw }

private fun backends(array: JSONArray): List<NativeBackendId> = List(array.length()) { backend(array.getString(it)) }

/** An externally tagged enum value: `"Unit"` or `{"Variant": {...}}` with exactly one key. */
private fun tagged(raw: Any): Pair<String, JSONObject?> =
    when (raw) {
        is String -> {
            raw to null
        }

        is JSONObject -> {
            require(raw.length() == 1)
            val tag = raw.keys().next()
            tag to raw.getJSONObject(tag)
        }

        else -> {
            throw IllegalArgumentException("not an enum value")
        }
    }

@Suppress("CyclomaticComplexMethod", "LongMethod") // One branch per wire variant.
private fun outcome(raw: Any): NativeBootOutcome {
    val (tag, body) = tagged(raw)
    if (body == null) {
        require(tag == "NoBackend")
        return NativeBootOutcome.NoBackend
    }
    return when (tag) {
        "Selected" -> {
            NativeBootOutcome.Selected(
                backend(body.getString("backend")),
                NativeSelectionMode.entries.first { it.name == body.getString("mode") },
                body.optBool("filesystem_hiding"),
            )
        }

        "Conflict" -> {
            NativeBootOutcome.Conflict(
                backends(body.getJSONArray("installed")),
                backends(body.getJSONArray("live")),
                backends(body.getJSONArray("staged")),
            )
        }

        "AwaitingAuthentication" -> {
            NativeBootOutcome.AwaitingAuthentication(body.getString("detail"))
        }

        "KpmUnverifiable" -> {
            NativeBootOutcome.KpmUnverifiable(body.getString("detail"))
        }

        "KpmIncompatible" -> {
            NativeBootOutcome.KpmIncompatible(
                found = body.optNullable("found") as String?,
                minimum = body.getString("minimum"),
                detail = body.getString("detail"),
            )
        }

        "CapacityExceeded" -> {
            NativeBootOutcome.CapacityExceeded(backend(body.getString("backend")), parseNativeCapacityReport(body.getJSONObject("report")))
        }

        "UnsupportedKernel" -> {
            NativeBootOutcome.UnsupportedKernel(backend(body.getString("backend")), body.getString("uname"), body.getString("detail"))
        }

        "MissingArtifact" -> {
            NativeBootOutcome.MissingArtifact(backend(body.getString("backend")), body.getString("path"))
        }

        "LoadFailed" -> {
            val tail = body.getJSONArray("dmesg_tail")
            NativeBootOutcome.LoadFailed(
                backend = backend(body.getString("backend")),
                exit = body.optNullable("exit")?.let { (it as Number).toInt() },
                stderr = body.getString("stderr"),
                dmesgTail = List(tail.length()) { tail.getString(it) },
                kprobes = body.optBool("kprobes"),
                kretprobes = body.optBool("kretprobes"),
                signatureRejected = body.getBoolean("signature_rejected"),
            )
        }

        "DeliveryFailed" -> {
            NativeBootOutcome.DeliveryFailed(backend(body.getString("backend")), body.getString("detail"))
        }

        "ForeignOwner" -> {
            NativeBootOutcome.ForeignOwner(body.getLong("id"))
        }

        "Unknown" -> {
            NativeBootOutcome.Unknown(body.getString("detail"))
        }

        else -> {
            throw IllegalArgumentException("unknown outcome $tag")
        }
    }
}

private fun kmodReport(json: JSONObject) =
    KmodBootReport(
        kprobes = json.optBool("kprobes"),
        kretprobes = json.optBool("kretprobes"),
        gkiVariant = json.getString("gki_variant"),
        version = json.getString("version"),
        uname = json.getString("uname"),
    )

private fun zygiskTargets(raw: Any): ZygiskTargetsState {
    val (tag, body) = tagged(raw)
    return when (tag) {
        "NotInstalled" -> ZygiskTargetsState.NotInstalled
        "Pending" -> ZygiskTargetsState.Pending
        "Published" -> ZygiskTargetsState.Published
        "Retired" -> ZygiskTargetsState.Retired
        "RetireFailed" -> ZygiskTargetsState.RetireFailed(requireNotNull(body).getString("detail"))
        else -> throw IllegalArgumentException("unknown zygisk targets $tag")
    }.also { require((body == null) == (tag != "RetireFailed")) }
}

private fun ports(json: JSONObject) =
    PortsBootStatus(
        source = PortsBootSource.entries.first { it.name == json.getString("source") },
        ipv4 = FamilyOutcome.entries.first { it.name == json.getString("ipv4") },
        ipv6 = FamilyOutcome.entries.first { it.name == json.getString("ipv6") },
        targetCount = json.getInt("target_count"),
        netdReady = json.optBool("netd_ready"),
        detail = json.getString("detail"),
        atMs = json.getLong("at_ms"),
    )

// ── Adapters for the per-backend diagnoses ───────────────────────────────

/**
 * The KPM view of core's decision, for the KPM card and its classifier. Null-equivalent
 * (runtime Unknown, no boot id) when core reported nothing or the outcome is not about KPM.
 */
@Suppress("CyclomaticComplexMethod")
internal fun NativeBootStatus?.kpmLoadStatus(): KpmLoadStatus {
    val boot = this ?: return KPM_STATUS_NONE

    fun status(
        runtime: KpmRuntime,
        loaded: Boolean?,
        reason: KpmFailureReason,
        detail: String? = null,
        filesystemHiding: Boolean? = null,
        unameR: String? = null,
    ) = KpmLoadStatus(
        timestamp = boot.decidedAtMs / MILLIS_PER_SECOND,
        bootId = boot.bootId,
        unameR = unameR,
        runtime = runtime,
        loaded = loaded,
        filesystemHiding = filesystemHiding,
        reason = reason,
        detail = detail?.ifBlank { null },
    )
    return when (val outcome = boot.outcome) {
        is NativeBootOutcome.Selected -> {
            if (outcome.backend == NativeBackendId.Kpm) {
                status(KpmRuntime.Activator, true, KpmFailureReason.Ok, filesystemHiding = outcome.filesystemHiding)
            } else {
                KPM_STATUS_NONE
            }
        }

        is NativeBootOutcome.Conflict -> {
            if (NativeBackendId.Kpm in outcome.backends) {
                status(KpmRuntime.Conflict, false, KpmFailureReason.ConflictingBackend)
            } else {
                KPM_STATUS_NONE
            }
        }

        is NativeBootOutcome.AwaitingAuthentication -> {
            status(KpmRuntime.Apatch, false, KpmFailureReason.AwaitingSuperkey, outcome.detail)
        }

        is NativeBootOutcome.KpmUnverifiable -> {
            status(KpmRuntime.Unknown, null, KpmFailureReason.Unknown, outcome.detail)
        }

        is NativeBootOutcome.KpmIncompatible -> {
            status(KpmRuntime.Unknown, true, KpmFailureReason.ActivationFailed, outcome.detail)
        }

        // The KPM runs; the config just does not fit. The dashboard's capacity issue owns
        // the wording, so the KPM card must not also report a load problem.
        is NativeBootOutcome.CapacityExceeded -> {
            kpmOnly(outcome.backend) { status(KpmRuntime.Activator, true, KpmFailureReason.Ok) }
        }

        is NativeBootOutcome.UnsupportedKernel -> {
            kpmOnly(
                outcome.backend,
            ) { status(KpmRuntime.Activator, false, KpmFailureReason.UnsupportedKernel, outcome.detail, unameR = outcome.uname) }
        }

        is NativeBootOutcome.MissingArtifact -> {
            kpmOnly(outcome.backend) { status(KpmRuntime.Activator, false, KpmFailureReason.MissingKpm, outcome.path) }
        }

        is NativeBootOutcome.LoadFailed -> {
            kpmOnly(outcome.backend) { status(KpmRuntime.Activator, false, KpmFailureReason.LoadFailed, outcome.stderr) }
        }

        is NativeBootOutcome.DeliveryFailed -> {
            kpmOnly(outcome.backend) { status(KpmRuntime.Activator, true, KpmFailureReason.ActivationFailed, outcome.detail) }
        }

        is NativeBootOutcome.ForeignOwner, NativeBootOutcome.NoBackend, is NativeBootOutcome.Unknown -> {
            KPM_STATUS_NONE
        }
    }
}

private inline fun kpmOnly(
    backend: NativeBackendId,
    status: () -> KpmLoadStatus,
): KpmLoadStatus = if (backend == NativeBackendId.Kpm) status() else KPM_STATUS_NONE

private const val MILLIS_PER_SECOND = 1000L

internal val KPM_STATUS_NONE =
    KpmLoadStatus(
        timestamp = null,
        bootId = null,
        unameR = null,
        runtime = KpmRuntime.Unknown,
        loaded = null,
        filesystemHiding = null,
        reason = KpmFailureReason.Unknown,
        detail = null,
    )

/**
 * The kmod view of core's decision, for the kmod card and [classifyKmodProblem]. Null when
 * core reported nothing about the kmod this boot.
 */
internal fun NativeBootStatus?.kmodLoadStatus(currentBootId: String): KmodLoadStatus? {
    val boot = this ?: return null
    val report = boot.kmod
    val outcome = boot.outcome
    val failed = (outcome as? NativeBootOutcome.LoadFailed)?.takeIf { it.backend == NativeBackendId.Kmod }
    val selected = (outcome as? NativeBootOutcome.Selected)?.takeIf { it.backend == NativeBackendId.Kmod }
    val missing = (outcome as? NativeBootOutcome.MissingArtifact)?.takeIf { it.backend == NativeBackendId.Kmod }
    if (report == null && failed == null && selected == null && missing == null) return null

    fun flag(value: Boolean?): String? = value?.let { if (it) "y" else "n" }
    return KmodLoadStatus(
        timestamp = boot.decidedAtMs / MILLIS_PER_SECOND,
        bootId = boot.bootId,
        unameR = report?.uname?.ifBlank { null },
        gkiVariant = report?.gkiVariant?.ifBlank { null },
        kmodVersion = report?.version?.ifBlank { null },
        rootManager = null,
        kprobes = flag(failed?.kprobes ?: report?.kprobes),
        kretprobes = flag(failed?.kretprobes ?: report?.kretprobes),
        insmodExit = failed?.exit ?: if (missing != null) EXIT_MISSING else null,
        loaded = selected != null,
        insmodStderr = failed?.stderr?.ifBlank { null } ?: missing?.let { "missing ${it.path}" },
        dmesgTail = failed?.dmesgTail?.joinToString("\n")?.ifBlank { null },
        freshForCurrentBoot = boot.isFreshFor(currentBootId),
        filesystemHiding = selected?.filesystemHiding,
        signatureRejected = failed?.signatureRejected,
    )
}

private const val EXIT_MISSING = 127
