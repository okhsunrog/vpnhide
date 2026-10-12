package dev.okhsunrog.vpnhide

/** Internal provenance; does not change serialized dashboard/bundle payloads. */
internal data class RootProjection<T>(
    val observationId: Long,
    val generation: Long,
    val value: T,
)

/** A projection can only be assembled with diagnostics using the same root observation. */
internal fun <T> RootProjection<T>.belongsTo(root: RootSnapshot?): Boolean =
    root != null && observationId == root.observationId && generation == root.generation

internal fun rootObservationInvalidatesDependents(
    previous: ObservationState<RootSnapshot>,
    next: ObservationState<RootSnapshot>,
): Boolean =
    next.generation != previous.generation ||
        (previous.attempted && previous.active == null && next.active != null)

internal fun rootObservationInvalidatesInventory(
    previous: ObservationState<RootSnapshot>,
    next: ObservationState<RootSnapshot>,
): Boolean =
    rootInventoryChanged(previous.lastGood?.value, next.lastGood?.value) ||
        (previous.lastGood == null && rootObservationInvalidatesDependents(previous, next))

/** Config/statistics refreshes need not rescan icons when the package inventory is unchanged. */
internal fun rootInventoryChanged(
    previous: RootSnapshot?,
    next: RootSnapshot?,
): Boolean =
    previous != null && next != null &&
        (previous.sections["pm_packages"] != next.sections["pm_packages"] || previous.sections["pm_users"] != next.sections["pm_users"])
