package dev.okhsunrog.vpnhide

import android.content.Context
import android.content.pm.PackageManager
import android.content.res.Resources

/**
 * Wording for a [DashboardIssue]. The decision of *whether* an issue exists is
 * [dashboardIssues]; everything here is presentation, which is why it is the
 * only half that needs a [Context].
 *
 * Split per issue group so each `when` stays small — one 28-branch dispatch
 * would be a complexity hotspot for no benefit.
 */
internal fun DashboardIssue.toMessage(
    context: Context,
    res: Resources,
): DashboardMessage =
    when (this) {
        is DashboardIssue.Native -> nativeMessage(res)
        is DashboardIssue.Lsposed -> lsposedMessage(context, res)
        is DashboardIssue.Module -> moduleMessage(res)
        is DashboardIssue.Target -> targetMessage(res)
        is DashboardIssue.Environment -> environmentMessage(res)
        is DashboardIssue.Protection -> protectionMessage(res)
    }

private fun err(
    text: String,
    downloadArtifact: String? = null,
) = DashboardMessage(DashboardMessageSeverity.ERROR, text, downloadArtifact = downloadArtifact)

private fun warn(
    text: String,
    downloadArtifact: String? = null,
) = DashboardMessage(DashboardMessageSeverity.WARNING, text, downloadArtifact = downloadArtifact)

private fun info(
    text: String,
    action: DashboardMessageAction? = null,
) = DashboardMessage(DashboardMessageSeverity.INFO, text, action)

private fun DashboardIssue.Native.nativeMessage(res: Resources): DashboardMessage =
    when (this) {
        DashboardIssue.KpmStandaloneInstall -> {
            err(res.getString(R.string.dashboard_issue_kpm_standalone_install), "vpnhide-kpm.zip")
        }

        DashboardIssue.NoNativeBackend -> {
            err(res.getString(R.string.dashboard_issue_no_native))
        }

        is DashboardIssue.BetterBackendAvailable -> {
            info(
                res.getString(
                    when (backend) {
                        SuggestedBackend.Kmod -> R.string.dashboard_issue_kmod_capable_but_zygisk
                        SuggestedBackend.Kpm -> R.string.dashboard_issue_kpm_capable_but_zygisk
                    },
                    artifact,
                ),
            )
        }

        DashboardIssue.NativeConflictKernel -> {
            err(res.getString(R.string.dashboard_issue_native_conflict_kernel))
        }

        DashboardIssue.MultipleNativeActive -> {
            warn(res.getString(R.string.dashboard_issue_multiple_native))
        }

        DashboardIssue.NativeConflictDeferred -> {
            warn(res.getString(R.string.dashboard_issue_native_conflict_deferred))
        }

        DashboardIssue.KpmAwaitingSuperkey -> {
            warn(res.getString(R.string.dashboard_issue_kpm_awaiting_superkey))
        }
    }

private fun DashboardIssue.Lsposed.lsposedMessage(
    context: Context,
    res: Resources,
): DashboardMessage =
    when (this) {
        DashboardIssue.LsposedNotInstalled -> {
            err(res.getString(R.string.dashboard_issue_lsposed_not_installed))
        }

        DashboardIssue.LsposedNeedsReboot -> {
            err(res.getString(R.string.dashboard_issue_reboot))
        }

        DashboardIssue.LsposedConfigUnreadable -> {
            err(res.getString(R.string.dashboard_issue_lsposed_config_unreadable))
        }

        DashboardIssue.LsposedNotEnabled -> {
            err(res.getString(R.string.dashboard_issue_lsposed_not_enabled))
        }

        DashboardIssue.LsposedNoSystemScope -> {
            err(res.getString(R.string.dashboard_issue_lsposed_no_system_scope))
        }

        is DashboardIssue.LsposedExtraScope -> {
            // Extra entries work, they are only cosmetic noise — warn, don't error.
            warn(
                res.getString(
                    R.string.dashboard_issue_lsposed_extra_scope,
                    entries.joinToString(", ") { resolveScopeEntryLabel(context, it) },
                ),
            )
        }

        is DashboardIssue.LsposedFieldRename -> {
            err(res.getString(R.string.dashboard_issue_lsposed_field_rename, fields, sdkLabel))
        }

        is DashboardIssue.LsposedInstallFailures -> {
            err(res.getString(R.string.dashboard_issue_lsposed_install_failures, detail))
        }
    }

private fun DashboardIssue.Module.moduleMessage(res: Resources): DashboardMessage =
    when (this) {
        is DashboardIssue.ModuleVersionMismatch -> {
            // A version gap is a warning: the modules keep working, the user just
            // needs to update the lagging side.
            warn(
                buildModuleVersionIssue(
                    res = res,
                    kind = mismatch.kind,
                    moduleVersion = mismatch.moduleVersion,
                    appVersion = mismatch.appVersion,
                    recommendedArtifact = recommendedArtifact,
                ),
                downloadArtifact = downloadArtifact,
            )
        }

        // Text and artifact were resolved with the module's card diagnosis; see
        // DashboardIssue.ModuleBroken.
        is DashboardIssue.ModuleBroken -> {
            err(problem.text, problem.downloadArtifact)
        }

        is DashboardIssue.ModuleNeedsReboot -> {
            warn(res.getString(R.string.dashboard_issue_module_reboot_to_activate, kind.displayName))
        }
    }

private fun DashboardIssue.Target.targetMessage(res: Resources): DashboardMessage =
    when (this) {
        DashboardIssue.NoTargets -> {
            info(res.getString(R.string.dashboard_issue_no_targets))
        }

        DashboardIssue.PortsNoObservers -> {
            info(res.getString(R.string.dashboard_issue_ports_no_observers))
        }

        is DashboardIssue.PortsRulesInactive -> {
            warn(
                if (failureDetail == null) {
                    res.getString(R.string.dashboard_issue_ports_rules_inactive)
                } else {
                    res.getString(R.string.dashboard_issue_ports_apply_failed, failureDetail)
                },
            )
        }
    }

private fun DashboardIssue.Environment.environmentMessage(res: Resources): DashboardMessage =
    when (this) {
        is DashboardIssue.FilesystemHidingPending -> {
            warn(
                res.getString(
                    when {
                        enabling && zygisk -> R.string.dashboard_issue_filesystem_hiding_pending_enable_zygisk
                        enabling -> R.string.dashboard_issue_filesystem_hiding_pending_enable
                        zygisk -> R.string.dashboard_issue_filesystem_hiding_pending_disable_zygisk
                        else -> R.string.dashboard_issue_filesystem_hiding_pending_disable
                    },
                ),
            )
        }

        is DashboardIssue.FilesystemHidingBootError -> {
            err(res.getString(R.string.dashboard_issue_filesystem_hiding_boot_error, detail))
        }

        DashboardIssue.FilesystemHidingSetupError -> {
            err(res.getString(R.string.dashboard_issue_filesystem_hiding_setup_error))
        }

        DashboardIssue.DebugLoggingOn -> {
            info(res.getString(R.string.dashboard_issue_debug_logging_on))
        }

        DashboardIssue.AgentBridgeOn -> {
            info(res.getString(R.string.dashboard_issue_agent_bridge_on))
        }

        DashboardIssue.SelinuxPermissive -> {
            warn(res.getString(R.string.dashboard_issue_selinux_permissive))
        }

        is DashboardIssue.InstalledInMultipleProfiles -> {
            warn(res.getString(R.string.dashboard_issue_self_multi_profile, profileCount))
        }
    }

private fun DashboardIssue.Protection.protectionMessage(res: Resources): DashboardMessage =
    when (this) {
        is DashboardIssue.PartialHooks -> {
            // No reinstall fixes a kernel that renamed or dropped a function, so
            // this is a warning rather than an error.
            warn(
                res.getString(
                    R.string.dashboard_issue_native_partial_hooks,
                    installed,
                    expected,
                    missing.joinToString(", ") { it.hookName },
                ),
            )
        }

        is DashboardIssue.LsposedVersionMismatch -> {
            val text = res.getString(R.string.dashboard_issue_version_mismatch, runningVersion, appVersion)
            if (degraded) warn(text) else info(text)
        }

        DashboardIssue.ChecksFailed -> {
            DashboardMessage(
                DashboardMessageSeverity.WARNING,
                res.getString(R.string.dashboard_issue_checks_failed),
                DashboardMessageAction.OpenDiagnostics,
            )
        }
    }

/** Brand names, not localized — the same spellings the module cards use. */
private val FlashableModuleKind.displayName: String
    get() =
        when (this) {
            FlashableModuleKind.Kmod -> "kmod"
            FlashableModuleKind.Builtin -> "Built-in"
            FlashableModuleKind.Kpm -> "KPM"
            FlashableModuleKind.Zygisk -> "Zygisk"
            FlashableModuleKind.Ports -> "Ports"
        }

// Complexity is inherent: three per-kind string tables (older / newer / equal),
// one arm per flashable module in each. Adding the built-in backend nudged the
// branch count past the threshold, but the shape is flat dispatch, not tangled
// control flow.
@Suppress("CyclomaticComplexMethod")
private fun buildModuleVersionIssue(
    res: Resources,
    kind: FlashableModuleKind,
    moduleVersion: String,
    appVersion: String,
    // Only meaningful for FlashableModuleKind.Kmod: the GKI-specific zip the
    // kernel recommends, so an "update the module" nudge can name the exact
    // file instead of sending the user back to KernelSU/Magisk to guess
    // which variant they originally flashed (issue #225).
    recommendedArtifact: String? = null,
): String =
    when (compareSemver(normalizeVersion(moduleVersion), normalizeVersion(appVersion))) {
        null, 0 -> {
            res.getString(
                when (kind) {
                    FlashableModuleKind.Kmod -> R.string.dashboard_issue_kmod_version_mismatch
                    FlashableModuleKind.Builtin -> R.string.dashboard_issue_builtin_version_mismatch
                    FlashableModuleKind.Kpm -> R.string.dashboard_issue_kpm_version_mismatch
                    FlashableModuleKind.Zygisk -> R.string.dashboard_issue_zygisk_version_mismatch
                    FlashableModuleKind.Ports -> R.string.dashboard_issue_ports_version_mismatch
                },
                moduleVersion,
                appVersion,
            )
        }

        in Int.MIN_VALUE..-1 -> {
            if (kind == FlashableModuleKind.Kmod && recommendedArtifact != null) {
                res.getString(R.string.dashboard_issue_update_kmod_named, moduleVersion, appVersion, recommendedArtifact)
            } else {
                res.getString(
                    when (kind) {
                        FlashableModuleKind.Kmod -> R.string.dashboard_issue_update_kmod
                        FlashableModuleKind.Builtin -> R.string.dashboard_issue_update_builtin
                        FlashableModuleKind.Kpm -> R.string.dashboard_issue_update_kpm
                        FlashableModuleKind.Zygisk -> R.string.dashboard_issue_update_zygisk
                        FlashableModuleKind.Ports -> R.string.dashboard_issue_update_ports
                    },
                    moduleVersion,
                    appVersion,
                )
            }
        }

        else -> {
            res.getString(
                when (kind) {
                    FlashableModuleKind.Kmod -> R.string.dashboard_issue_update_app_for_kmod
                    FlashableModuleKind.Builtin -> R.string.dashboard_issue_update_app_for_builtin
                    FlashableModuleKind.Kpm -> R.string.dashboard_issue_update_app_for_kpm
                    FlashableModuleKind.Zygisk -> R.string.dashboard_issue_update_app_for_zygisk
                    FlashableModuleKind.Ports -> R.string.dashboard_issue_update_app_for_ports
                },
                moduleVersion,
                appVersion,
            )
        }
    }

/**
 * An LSPosed scope entry (`pkg` or `pkg/user`) as the user would recognise it.
 * Falls back to the package name when the label is unavailable — a scope entry
 * can outlive the app it names.
 */
private fun resolveScopeEntryLabel(
    context: Context,
    entry: String,
): String {
    if (entry == "system" || entry == "system/0") return "System Framework"

    val packageName = entry.substringBefore('/')
    val userId = entry.substringAfter('/', "")
    return try {
        val appInfo = context.packageManager.getApplicationInfo(packageName, 0)
        val appLabel =
            context.packageManager
                .getApplicationLabel(appInfo)
                .toString()
                .trim()
        when {
            appLabel.isEmpty() -> packageName
            userId.isNotEmpty() && userId != "0" -> "$appLabel ($userId)"
            else -> appLabel
        }
    } catch (_: PackageManager.NameNotFoundException) {
        packageName
    }
}
