package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ConfigCoordinatorView
import dev.okhsunrog.vpnhide.RootSnapshot

/** Wait for actor acceptance of the publication a root projection already joined. No I/O is owed. */
internal fun coreInputsJoined(
    root: RootSnapshot?,
    config: ConfigCoordinatorView,
): Boolean = root?.core?.let { coreInputsJoined(it, config) } ?: false

internal fun coreInputsJoined(
    publication: dev.okhsunrog.vpnhide.CoreSessionUpdate,
    config: ConfigCoordinatorView,
): Boolean {
    val state = publication.state ?: return false
    return publication.current && config.coreCurrent && state.epoch == config.coreEpoch &&
        state.configRevision == config.desiredRevision && publication.sequence <= config.coreSequence
}
