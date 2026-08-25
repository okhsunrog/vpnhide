package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.checks.CheckStatus
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/**
 * The honest per-check outcome, decided from the app's in-process probe result
 * combined with the root ground-truth differential (see [GroundTruthProbe]).
 *
 * Rather than a bare pass/fail, a clean app-view result is split by WHO hid the
 * VPN: the backend (root sees it, the app doesn't), SELinux (the app was
 * EACCES-blocked), or nothing at all (root also sees nothing). [NotMeasured] is
 * orthogonal — the probe produced no usable observation, so it votes for neither.
 */
@Serializable
sealed interface CheckOutcome {
    /** App saw VPN-shaped data — a real leak. */
    @Serializable
    @SerialName("leak")
    data object Leak : CheckOutcome

    /** App saw no VPN and root did: the active backend hid it. (Attribution to a
     * specific hook is layered on later from the coverage map / counters.) */
    @Serializable
    @SerialName("hidden_backend")
    data object HiddenByBackend : CheckOutcome

    /** The app's read was denied by SELinux — SELinux hid it, not a backend hook. */
    @Serializable
    @SerialName("hidden_selinux")
    data object HiddenBySelinux : CheckOutcome

    /** Nothing VPN-shaped on this surface for anyone (root also saw nothing). */
    @Serializable
    @SerialName("nothing_to_leak")
    data object NothingToLeak : CheckOutcome

    /** No usable observation: the probe couldn't run, or no ground truth. */
    @Serializable
    @SerialName("not_measured")
    data class NotMeasured(
        val reason: NotMeasuredReason,
    ) : CheckOutcome
}

@Serializable
enum class NotMeasuredReason(
    val token: String,
) {
    NoNetworkPermission("not_measured_no_network"),
    NoGroundTruth("not_measured_no_ground_truth"),
}

/**
 * Classify one native check from its app-view result and the root ground truth.
 *
 * Priority: a leak is a leak; an empty ground truth means nothing-to-leak even
 * when the app read was SELinux-blocked (the block is moot when there is nothing
 * on that surface to hide).
 */
fun classifyNativeOutcome(
    appView: CheckOutput,
    groundTruth: CheckOutput?,
): CheckOutcome =
    when (appView.status) {
        CheckStatus.FAIL -> {
            CheckOutcome.Leak
        }

        CheckStatus.NETWORK_BLOCKED -> {
            CheckOutcome.NotMeasured(NotMeasuredReason.NoNetworkPermission)
        }

        // App saw no VPN (PASS or SELINUX_BLOCKED): who hid it? Ask the ground truth.
        CheckStatus.PASS, CheckStatus.SELINUX_BLOCKED -> {
            when (groundTruth?.status) {
                CheckStatus.PASS -> {
                    CheckOutcome.NothingToLeak // root also clean → nothing there
                }

                CheckStatus.FAIL -> {
                    if (appView.status == CheckStatus.SELINUX_BLOCKED) {
                        CheckOutcome.HiddenBySelinux
                    } else {
                        CheckOutcome.HiddenByBackend
                    }
                }

                // No root result, or root itself blocked/limited → cannot attribute.
                else -> {
                    CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth)
                }
            }
        }
    }

/**
 * Classify one Java-level check. Java checks are framework IPC with no root
 * differential — but the diagnostics only run once the self-in-tunnel gate has
 * confirmed a VPN is up AND this app is routed through it. So a VPN artifact is
 * always present for a VPN-revealing check to surface; a clean app-view therefore
 * means the LSPosed hook removed it, and a dirty one is a real leak. There is no
 * "nothing to leak" case on this layer.
 *
 * [NotMeasured] is only a defensive edge for a probe that could not actually run
 * (clean == null): a hidden framework method that reflection can't reach, or no
 * active network — states the gate makes near-impossible but that we refuse to
 * paint as a backend success.
 */
fun classifyJavaOutcome(clean: Boolean?): CheckOutcome =
    when (clean) {
        false -> CheckOutcome.Leak
        true -> CheckOutcome.HiddenByBackend
        null -> CheckOutcome.NotMeasured(NotMeasuredReason.NoGroundTruth)
    }

/** Stable wire/log token for an outcome (for the agent bridge + debug export). */
fun CheckOutcome.token(): String =
    when (this) {
        CheckOutcome.Leak -> "leak"
        CheckOutcome.HiddenByBackend -> "hidden_backend"
        CheckOutcome.HiddenBySelinux -> "hidden_selinux"
        CheckOutcome.NothingToLeak -> "nothing_to_leak"
        is CheckOutcome.NotMeasured -> reason.token
    }
