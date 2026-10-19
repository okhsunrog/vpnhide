package dev.okhsunrog.vpnhide

internal enum class KpmRuntime {
    Activator,
    KpatchNext,
    Apatch,
    Conflict,
    Unknown,
}

internal enum class KpmFailureReason {
    Ok,
    ConflictingBackend,
    MissingKpm,
    AwaitingSuperkey,
    UnsupportedKernel,
    ActivationFailed,
    LoadFailed,
    Unknown,
}

/** The KPM view of core's once-per-boot native decision ([NativeBootStatus.kpmLoadStatus]). */
internal data class KpmLoadStatus(
    val timestamp: Long?,
    val bootId: String?,
    val unameR: String?,
    val runtime: KpmRuntime,
    val loaded: Boolean?,
    val filesystemHiding: Boolean?,
    val reason: KpmFailureReason,
    val detail: String?,
) {
    fun isFreshFor(currentBootId: String): Boolean = !bootId.isNullOrEmpty() && bootId == currentBootId.trim()
}
