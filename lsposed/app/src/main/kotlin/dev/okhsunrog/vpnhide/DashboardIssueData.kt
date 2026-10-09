package dev.okhsunrog.vpnhide

import kotlinx.serialization.Serializable

/** Stable, locale-independent reason alongside the existing rendered message. */
@Serializable
internal data class DashboardIssueReason(
    val kind: String,
    val parameters: Map<String, String> = emptyMap(),
)

// Exhaustive vocabulary mapping, not another issue classifier. Keep identifiers
// explicit so moving/renaming a Kotlin type does not change exported reasons.
@Suppress("CyclomaticComplexMethod", "LongMethod")
internal fun DashboardIssue.toReason(): DashboardIssueReason =
    when (this) {
        DashboardIssue.KpmStandaloneInstall -> {
            DashboardIssueReason("kpm_standalone_install")
        }

        DashboardIssue.NoNativeBackend -> {
            DashboardIssueReason("no_native_backend")
        }

        is DashboardIssue.BetterBackendAvailable -> {
            DashboardIssueReason("better_backend_available", mapOf("artifact" to artifact))
        }

        DashboardIssue.NativeConflictKernel -> {
            DashboardIssueReason("native_conflict_kernel")
        }

        DashboardIssue.BuiltinCompanionMissing -> {
            DashboardIssueReason("builtin_companion_missing")
        }

        DashboardIssue.RedundantKmodWithBuiltin -> {
            DashboardIssueReason("redundant_kmod_with_builtin")
        }

        DashboardIssue.MultipleNativeActive -> {
            DashboardIssueReason("multiple_native_active")
        }

        DashboardIssue.NativeConflictDeferred -> {
            DashboardIssueReason("native_conflict_deferred")
        }

        DashboardIssue.KpmAwaitingSuperkey -> {
            DashboardIssueReason("kpm_awaiting_superkey")
        }

        DashboardIssue.LsposedNotInstalled -> {
            DashboardIssueReason("lsposed_not_installed")
        }

        DashboardIssue.LsposedNeedsReboot -> {
            DashboardIssueReason("lsposed_needs_reboot")
        }

        DashboardIssue.LsposedConfigUnreadable -> {
            DashboardIssueReason("lsposed_config_unreadable")
        }

        DashboardIssue.LsposedNotEnabled -> {
            DashboardIssueReason("lsposed_not_enabled")
        }

        DashboardIssue.LsposedNoSystemScope -> {
            DashboardIssueReason("lsposed_no_system_scope")
        }

        is DashboardIssue.LsposedExtraScope -> {
            DashboardIssueReason("lsposed_extra_scope", mapOf("entries" to entries.joinToString(",")))
        }

        is DashboardIssue.LsposedFieldRename -> {
            DashboardIssueReason("lsposed_field_rename", mapOf("fields" to fields, "sdk" to sdkLabel))
        }

        is DashboardIssue.LsposedInstallFailures -> {
            DashboardIssueReason("lsposed_install_failures", mapOf("detail" to detail))
        }

        is DashboardIssue.ModuleVersionMismatch -> {
            DashboardIssueReason(
                "module_version_mismatch",
                mapOf(
                    "component" to mismatch.kind.reasonId(),
                    "moduleVersion" to mismatch.moduleVersion,
                    "appVersion" to mismatch.appVersion,
                ),
            )
        }

        is DashboardIssue.ModuleBroken -> {
            DashboardIssueReason("module_broken", mapOf("artifact" to problem.downloadArtifact.orEmpty()))
        }

        is DashboardIssue.ModuleNeedsReboot -> {
            DashboardIssueReason("module_needs_reboot", mapOf("component" to kind.reasonId()))
        }

        DashboardIssue.NoTargets -> {
            DashboardIssueReason("no_targets")
        }

        DashboardIssue.PortsNoObservers -> {
            DashboardIssueReason("ports_no_observers")
        }

        is DashboardIssue.PortsRulesInactive -> {
            DashboardIssueReason("ports_rules_inactive", mapOf("detail" to failureDetail.orEmpty()))
        }

        is DashboardIssue.FilesystemHidingPending -> {
            DashboardIssueReason(
                "filesystem_hiding_pending",
                mapOf("enabling" to enabling.toString(), "zygisk" to zygisk.toString()),
            )
        }

        is DashboardIssue.FilesystemHidingBootError -> {
            DashboardIssueReason("filesystem_hiding_boot_error", mapOf("detail" to detail))
        }

        DashboardIssue.FilesystemHidingSetupError -> {
            DashboardIssueReason("filesystem_hiding_setup_error")
        }

        DashboardIssue.DebugLoggingOn -> {
            DashboardIssueReason("debug_logging_on")
        }

        DashboardIssue.AgentBridgeOn -> {
            DashboardIssueReason("agent_bridge_on")
        }

        DashboardIssue.SelinuxPermissive -> {
            DashboardIssueReason("selinux_permissive")
        }

        is DashboardIssue.InstalledInMultipleProfiles -> {
            DashboardIssueReason(
                "installed_in_multiple_profiles",
                mapOf("count" to profileCount.toString()),
            )
        }

        is DashboardIssue.PartialHooks -> {
            DashboardIssueReason(
                "partial_hooks",
                mapOf(
                    "installed" to installed.toString(),
                    "expected" to expected.toString(),
                    "missing" to missing.joinToString(",") { it.hookName },
                ),
            )
        }

        is DashboardIssue.LsposedVersionMismatch -> {
            DashboardIssueReason(
                "lsposed_version_mismatch",
                mapOf(
                    "component" to "lsposed",
                    "runningVersion" to runningVersion,
                    "appVersion" to appVersion,
                    "degraded" to degraded.toString(),
                ),
            )
        }

        DashboardIssue.ChecksFailed -> {
            DashboardIssueReason("checks_failed")
        }
    }

private fun FlashableModuleKind.reasonId(): String =
    when (this) {
        FlashableModuleKind.Kmod -> "kmod"
        FlashableModuleKind.Builtin -> "builtin"
        FlashableModuleKind.Kpm -> "kpm"
        FlashableModuleKind.Zygisk -> "zygisk"
        FlashableModuleKind.Ports -> "ports"
    }
