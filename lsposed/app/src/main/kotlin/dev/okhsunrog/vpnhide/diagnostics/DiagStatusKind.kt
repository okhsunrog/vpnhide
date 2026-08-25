package dev.okhsunrog.vpnhide.diagnostics

/**
 * The visual bucket a check falls into on the Diagnostics screen — the pure
 * decision behind each card's dot + word + colour, split out so it is unit-tested
 * without Compose. The Composable only maps a [DiagStatusKind] to a localized
 * label and theme colour; it never re-derives which bucket a check belongs in.
 *
 * The card colour tracks *current reality*: green when the VPN is hidden (by a
 * backend hook OR SELinux) or there was nothing to leak, red on a real leak,
 * neutral when a probe couldn't run — so a normal enforcing device reads all
 * green. The backend-vs-SELinux distinction rides on the dot/word ([Selinux] vs
 * [Ok]), and [NothingToLeak] is set apart from [Ok] so "nothing on this surface"
 * never masquerades as active protection.
 */
internal enum class DiagStatusKind { Ok, Leak, NothingToLeak, Selinux, NotMeasured }

/** The screen bucket for a classified [CheckOutcome]. */
internal fun CheckOutcome.diagStatusKind(): DiagStatusKind =
    when (this) {
        CheckOutcome.Leak -> DiagStatusKind.Leak
        CheckOutcome.HiddenByBackend -> DiagStatusKind.Ok
        CheckOutcome.NothingToLeak -> DiagStatusKind.NothingToLeak
        CheckOutcome.HiddenBySelinux -> DiagStatusKind.Selinux
        is CheckOutcome.NotMeasured -> DiagStatusKind.NotMeasured
    }

/** Counts the two headline categories for the Diagnostics summary line: vectors
 * the app is actively hidden on (backend or SELinux) and vectors still leaking. */
internal data class ProtectionCounts(
    val hidden: Int,
    val leaks: Int,
)

internal fun Iterable<CheckResult>.protectionCounts(): ProtectionCounts =
    ProtectionCounts(
        hidden = count { it.outcome is CheckOutcome.HiddenByBackend || it.outcome is CheckOutcome.HiddenBySelinux },
        leaks = count { it.outcome is CheckOutcome.Leak },
    )

/** Whether any native probe couldn't run for lack of network permission — the
 * signal behind the "network access disabled" banner. */
internal fun Iterable<CheckResult>.anyNetworkBlocked(): Boolean =
    any { (it.outcome as? CheckOutcome.NotMeasured)?.reason == NotMeasuredReason.NoNetworkPermission }
