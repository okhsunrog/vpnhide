package dev.okhsunrog.vpnhide.debug

import dev.okhsunrog.vpnhide.ModuleState
import dev.okhsunrog.vpnhide.RootCanonicalRead
import dev.okhsunrog.vpnhide.canonicalConfigJson
import dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticSnapshot
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement

/** Actor drafts and raw shell canonical sections cannot override this publication. */
internal fun autonomousExportCanonical(snapshot: AutonomousDiagnosticSnapshot): JsonElement? {
    val publication = snapshot.sources.core
    if (!publication.current) return null
    val canonical = publication.state?.canonical as? RootCanonicalRead.Available ?: return null
    return Json.parseToJsonElement(canonicalConfigJson(canonical.config))
}

/** Retain installation metadata without asserting historical runtime liveness. */
internal fun unverifiedExportRuntime(state: ModuleState): ModuleState =
    when (state) {
        is ModuleState.Installed -> state.copy(active = false, runtimeCheckable = false)
        else -> state
    }
