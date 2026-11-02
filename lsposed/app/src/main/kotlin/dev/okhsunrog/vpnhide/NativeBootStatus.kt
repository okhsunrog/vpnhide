package dev.okhsunrog.vpnhide

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/** Core's typed boot decision and forensic evidence; Kotlin performs no boot codec or selection. */
@Serializable
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
    /**
     * Installed 1.x modules core found at its boot scan (core.log has the same line).
     * The Dashboard reads the live root-snapshot list instead; this is the record
     * that survives without the app.
     */
    val obsoleteModules: List<String> = emptyList(),
) {
    fun isFreshFor(currentBootId: String): Boolean = bootId.isNotEmpty() && bootId == currentBootId.trim()
}

@Serializable
internal enum class NativeSelectionMode { Adopted, Loaded, Authorized }

@Serializable
internal sealed interface NativeBootOutcome {
    @Serializable
    @SerialName("selected")
    data class Selected(
        val backend: NativeBackendId,
        val mode: NativeSelectionMode,
        val filesystemHiding: Boolean?,
    ) : NativeBootOutcome

    @Serializable
    @SerialName("conflict")
    data class Conflict(
        val installed: List<NativeBackendId>,
        val live: List<NativeBackendId>,
        val staged: List<NativeBackendId>,
    ) : NativeBootOutcome {
        /** Every backend that took part in the conflict, in a stable order. */
        val backends: List<NativeBackendId> get() = (installed + live).distinct().sortedBy { it.ordinal }
    }

    @Serializable
    @SerialName("awaiting_authentication")
    data class AwaitingAuthentication(
        val detail: String,
    ) : NativeBootOutcome

    @Serializable
    @SerialName("kpm_unverifiable")
    data class KpmUnverifiable(
        val detail: String,
    ) : NativeBootOutcome

    /**
     * A VPN Hide KPM is live — loaded by core earlier, autoloaded by APatch or loaded by
     * hand — but core cannot drive it ([found] is too old / unrecognised, or it failed the
     * status handshake). Nothing was configured and no second copy was loaded.
     */
    @Serializable
    @SerialName("kpm_incompatible")
    data class KpmIncompatible(
        val found: String?,
        val minimum: String,
        val detail: String,
    ) : NativeBootOutcome

    /**
     * The saved config projects to more target UIDs than the selected backend holds (a
     * Repair or boot delivery). Core sent nothing: the backend keeps its previous targets.
     */
    @Serializable
    @SerialName("capacity_exceeded")
    data class CapacityExceeded(
        val backend: NativeBackendId,
        val report: NativeCapacityRefusal,
    ) : NativeBootOutcome

    @Serializable
    @SerialName("unsupported_kernel")
    data class UnsupportedKernel(
        val backend: NativeBackendId,
        val uname: String,
        val detail: String,
    ) : NativeBootOutcome

    @Serializable
    @SerialName("missing_artifact")
    data class MissingArtifact(
        val backend: NativeBackendId,
        val path: String,
    ) : NativeBootOutcome

    @Serializable
    @SerialName("load_failed")
    data class LoadFailed(
        val backend: NativeBackendId,
        val exit: Int?,
        val stderr: String,
        val dmesgTail: List<String>,
        val kprobes: Boolean?,
        val kretprobes: Boolean?,
        val signatureRejected: Boolean,
    ) : NativeBootOutcome

    @Serializable
    @SerialName("delivery_failed")
    data class DeliveryFailed(
        val backend: NativeBackendId,
        val detail: String,
    ) : NativeBootOutcome

    @Serializable
    @SerialName("foreign_owner")
    data class ForeignOwner(
        val id: Long,
    ) : NativeBootOutcome

    @Serializable
    @SerialName("no_backend")
    data object NoBackend : NativeBootOutcome

    @Serializable
    @SerialName("unknown")
    data class Unknown(
        val detail: String,
    ) : NativeBootOutcome
}

@Serializable
internal data class KmodBootReport(
    val kprobes: Boolean?,
    val kretprobes: Boolean?,
    val gkiVariant: String,
    val version: String,
    val uname: String,
)

@Serializable
internal sealed interface ZygiskTargetsState {
    @Serializable
    @SerialName("not_installed")
    data object NotInstalled : ZygiskTargetsState

    @Serializable
    @SerialName("pending")
    data object Pending : ZygiskTargetsState

    @Serializable
    @SerialName("published")
    data object Published : ZygiskTargetsState

    @Serializable
    @SerialName("retired")
    data object Retired : ZygiskTargetsState

    @Serializable
    @SerialName("retire_failed")
    data class RetireFailed(
        val detail: String,
    ) : ZygiskTargetsState
}

@Serializable
internal enum class PortsBootSource { Boot, BootRetry, Edit }

@Serializable
internal enum class FamilyOutcome { Confirmed, FailedKnown, Unknown, NotAttempted }

@Serializable
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
