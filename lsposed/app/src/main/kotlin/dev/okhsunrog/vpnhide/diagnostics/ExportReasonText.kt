package dev.okhsunrog.vpnhide.diagnostics

import androidx.annotation.StringRes
import dev.okhsunrog.vpnhide.R

@StringRes
internal fun exportReasonResource(reason: ExportReason): Int =
    when (reason) {
        ExportReason.CoreUnavailable -> R.string.export_reason_core
        ExportReason.Incomplete -> R.string.export_reason_incomplete
        ExportReason.NoCause -> R.string.export_reason_none
        ExportReason.KernelHooks -> R.string.export_reason_kernel_hooks
        ExportReason.KernelCompatibility -> R.string.export_reason_kernel_compat
        ExportReason.JavaHooks -> R.string.export_reason_java_hooks
        ExportReason.JavaAttach -> R.string.export_reason_java_attach
        ExportReason.Configuration -> R.string.export_reason_configuration
    }
