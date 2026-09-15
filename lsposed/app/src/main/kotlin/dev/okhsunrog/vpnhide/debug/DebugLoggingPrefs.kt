package dev.okhsunrog.vpnhide.debug

import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.CanonicalEdit
import dev.okhsunrog.vpnhide.CanonicalMutation
import dev.okhsunrog.vpnhide.CanonicalToggle
import dev.okhsunrog.vpnhide.CanonicalWriteResult
import dev.okhsunrog.vpnhide.OperationSource
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot

private fun canonicalFromSnapshot(snapshot: RootSnapshot?): CanonicalConfig? =
    snapshot
        ?.let { parseTargetsSnapshot(it).canonicalConfig }

/**
 * Canonical config is the only source of truth for debug logging state.
 *
 * Before a root snapshot exists this is `false`.
 */
internal fun debugFromCanonicalSnapshot(rootSnapshot: RootSnapshot?): Boolean =
    canonicalFromSnapshot(rootSnapshot)
        ?.debug
        ?: false

/**
 * Set debug on the canonical JSON and propagate it through the native activator.
 * SU commands may fail on unusual root states, so callers should run this from
 * an IO dispatcher and still tolerate a temporary mismatch if needed.
 */
internal suspend fun setDebugLoggingEnabled(
    enabled: Boolean,
    source: OperationSource = OperationSource.Ui,
): CanonicalWriteResult =
    CanonicalConfigRepository.commit(CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, enabled)), source = source))
