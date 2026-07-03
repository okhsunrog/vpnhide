package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.checks.CheckStatus

/**
 * The honest per-check outcome, decided from the app's in-process probe result
 * combined with the root ground-truth differential (see [GroundTruthProbe]).
 *
 * Unlike the legacy `passed: Boolean?`, a clean app-view result is split by WHO
 * hid the VPN: the backend (root sees it, the app doesn't), SELinux (the app was
 * EACCES-blocked), or nothing at all (root also sees nothing). [NotMeasured] is
 * orthogonal — the probe produced no usable observation, so it votes for neither.
 */
sealed interface CheckOutcome {
    /** App saw VPN-shaped data — a real leak. */
    data object Leak : CheckOutcome

    /** App saw no VPN and root did: the active backend hid it. (Attribution to a
     * specific hook is layered on later from the coverage map / counters.) */
    data object HiddenByBackend : CheckOutcome

    /** The app's read was denied by SELinux — SELinux hid it, not a backend hook. */
    data object HiddenBySelinux : CheckOutcome

    /** Nothing VPN-shaped on this surface for anyone (root also saw nothing). */
    data object NothingToLeak : CheckOutcome

    /** No usable observation: the probe couldn't run, or no ground truth. */
    data class NotMeasured(
        val reason: NotMeasuredReason,
    ) : CheckOutcome
}

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

/** Stable wire/log token for an outcome (for the agent bridge + debug export). */
fun CheckOutcome.token(): String =
    when (this) {
        CheckOutcome.Leak -> "leak"
        CheckOutcome.HiddenByBackend -> "hidden_backend"
        CheckOutcome.HiddenBySelinux -> "hidden_selinux"
        CheckOutcome.NothingToLeak -> "nothing_to_leak"
        is CheckOutcome.NotMeasured -> reason.token
    }
