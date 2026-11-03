package dev.okhsunrog.vpnhide.diagnostics

import androidx.annotation.StringRes
import dev.okhsunrog.vpnhide.R

@StringRes
internal fun exportReasonResource(reason: ExportReason): Int =
    when (reason) {
        ExportReason.CORE_UNAVAILABLE -> R.string.export_reason_core
        ExportReason.INCOMPLETE -> R.string.export_reason_incomplete
        ExportReason.KERNEL_HOOKS -> R.string.export_reason_kernel_hooks
        ExportReason.KERNEL_COMPATIBILITY -> R.string.export_reason_kernel_compat
        ExportReason.JAVA_HOOKS -> R.string.export_reason_java_hooks
        ExportReason.CONFIGURATION -> R.string.export_reason_configuration
    }
