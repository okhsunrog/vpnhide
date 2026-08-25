package dev.okhsunrog.vpnhide.settings

import android.content.res.Resources
import dev.okhsunrog.vpnhide.DashboardMessage
import dev.okhsunrog.vpnhide.DashboardMessageSeverity
import dev.okhsunrog.vpnhide.NativeBackendId
import dev.okhsunrog.vpnhide.R

internal fun filesystemHidingDashboardMessage(
    desiredEnabled: Boolean,
    sections: Map<String, String>,
    res: Resources,
): DashboardMessage? {
    val state = resolveFilesystemHidingState(desiredEnabled, sections)
    return when (state.status) {
        FilesystemHidingStatus.PendingEnable -> {
            DashboardMessage(
                DashboardMessageSeverity.WARNING,
                res.getString(
                    if (state.backend == NativeBackendId.Zygisk) {
                        R.string.dashboard_issue_filesystem_hiding_pending_enable_zygisk
                    } else {
                        R.string.dashboard_issue_filesystem_hiding_pending_enable
                    },
                ),
            )
        }

        FilesystemHidingStatus.PendingDisable -> {
            DashboardMessage(
                DashboardMessageSeverity.WARNING,
                res.getString(
                    if (state.backend == NativeBackendId.Zygisk) {
                        R.string.dashboard_issue_filesystem_hiding_pending_disable_zygisk
                    } else {
                        R.string.dashboard_issue_filesystem_hiding_pending_disable
                    },
                ),
            )
        }

        FilesystemHidingStatus.BootConfigError -> {
            DashboardMessage(
                DashboardMessageSeverity.ERROR,
                res.getString(
                    R.string.dashboard_issue_filesystem_hiding_boot_error,
                    state.errorDetail.orEmpty(),
                ),
            )
        }

        FilesystemHidingStatus.HookSetupError -> {
            DashboardMessage(
                DashboardMessageSeverity.ERROR,
                res.getString(R.string.dashboard_issue_filesystem_hiding_setup_error),
            )
        }

        FilesystemHidingStatus.Unavailable,
        FilesystemHidingStatus.Disabled,
        FilesystemHidingStatus.Active,
        -> {
            null
        }
    }
}
