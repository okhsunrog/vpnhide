package dev.okhsunrog.vpnhide

/** Whether an installed backend module still carries the payload core loads. */
internal enum class ArtifactState {
    Present,
    Missing,
    Unknown,
}

internal fun parseArtifactState(raw: String): ArtifactState =
    when (raw.trim()) {
        "present" -> ArtifactState.Present
        "missing" -> ArtifactState.Missing
        else -> ArtifactState.Unknown
    }

internal data class ModuleIntegrityProblem(
    val kind: FlashableModuleKind,
    /** The payload core expects in the module directory, as the user would look for it. */
    val artifactPath: String,
) {
    val reason: ModuleBrokenReason get() = ModuleBrokenReason.ArtifactMissing
}

internal fun classifyModuleIntegrity(
    kind: FlashableModuleKind,
    module: ModuleState,
    artifactState: ArtifactState,
    disabled: Boolean,
): ModuleIntegrityProblem? {
    val artifactPath = moduleArtifactPath(kind) ?: return null
    if (module !is ModuleState.Installed || disabled || artifactState != ArtifactState.Missing) return null
    return ModuleIntegrityProblem(kind, artifactPath)
}

/** Section prefix of a module the root snapshot inspects; null when there is no module directory. */
private fun sectionPrefix(kind: FlashableModuleKind): String? =
    when (kind) {
        FlashableModuleKind.Kmod -> "kmod"

        // The in-tree driver is part of the kernel image: nothing to install.
        FlashableModuleKind.Builtin -> null

        FlashableModuleKind.Kpm -> "kpm"

        FlashableModuleKind.Zygisk -> "zygisk"

        FlashableModuleKind.Core -> "core"
    }

/** The payload core loads from each backend module (the snapshot's `<prefix>_artifact_state`). */
internal fun moduleArtifactPath(kind: FlashableModuleKind): String? =
    when (kind) {
        FlashableModuleKind.Kmod -> "$KMOD_MODULE_DIR/vpnhide_kmod.ko"

        FlashableModuleKind.Kpm -> "$KPM_MODULE_DIR/vpnhide.kpm"

        FlashableModuleKind.Zygisk -> "$ZYGISK_MODULE_DIR/zygisk/"

        // Core missing its binary is not running, so it could not have answered the snapshot.
        FlashableModuleKind.Builtin, FlashableModuleKind.Core -> null
    }

/**
 * True when the module has an update staged in modules_update/ awaiting the next
 * reboot (the root manager swaps it into modules/ on boot). This is "installed,
 * reboot needed", not a corrupt install — callers suppress the integrity error
 * and surface a reboot warning instead. Absent snapshot field ⇒ false.
 */
internal fun modulePendingReboot(
    kind: FlashableModuleKind,
    module: ModuleState,
    sections: Map<String, String>,
): Boolean {
    val prefix = sectionPrefix(kind) ?: return false
    if (module !is ModuleState.Installed || sections["${prefix}_disabled"]?.trim() == "1") return false
    return sections["${prefix}_pending_update"]?.trim() == "1"
}

internal fun moduleIntegrityProblem(
    kind: FlashableModuleKind,
    module: ModuleState,
    sections: Map<String, String>,
): ModuleIntegrityProblem? {
    // A staged install pending reboot is not an integrity failure.
    if (modulePendingReboot(kind, module, sections)) return null
    val prefix = sectionPrefix(kind) ?: return null
    return classifyModuleIntegrity(
        kind = kind,
        module = module,
        artifactState = parseArtifactState(sections["${prefix}_artifact_state"].orEmpty()),
        disabled = sections["${prefix}_disabled"]?.trim() == "1",
    )
}

internal fun ModuleState.withBrokenReason(reason: ModuleBrokenReason?): ModuleState =
    if (this is ModuleState.Installed && reason != null) copy(brokenReason = reason) else this

internal fun ModuleState.withPendingReboot(pending: Boolean): ModuleState =
    if (this is ModuleState.Installed && pending) copy(pendingReboot = true) else this
