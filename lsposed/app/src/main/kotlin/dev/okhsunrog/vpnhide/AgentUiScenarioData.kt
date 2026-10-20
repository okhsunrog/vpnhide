package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.KpmProblemKind

/** Typed production diagnoses; never strings that imitate the production banners. */
internal val agentUiIssues: Map<String, DashboardIssue> =
    linkedMapOf(
        "kpm-runtime-missing" to DashboardIssue.ModuleBroken(ModuleProblem.Kpm(KpmProblemKind.NoKernelPatchRuntime)),
        "kpm-superkey-missing" to DashboardIssue.ModuleBroken(ModuleProblem.Kpm(KpmProblemKind.NeedsSuperkey)),
        "kpm-load-failed" to DashboardIssue.ModuleBroken(ModuleProblem.Kpm(KpmProblemKind.LoadFailed("Fixture: load failed"))),
        "kpm-unsupported-kernel" to DashboardIssue.ModuleBroken(ModuleProblem.Kpm(KpmProblemKind.UnsupportedKernel("4.4.0-fixture"))),
        "kmod-kprobes-missing" to DashboardIssue.ModuleBroken(ModuleProblem.Kmod(KmodProblemKind.KprobesMissing)),
        "kmod-signature-enforced" to DashboardIssue.ModuleBroken(ModuleProblem.Kmod(KmodProblemKind.SignatureEnforced)),
        "native-conflict" to DashboardIssue.NativeConflictKernel,
        "native-missing" to DashboardIssue.NoNativeBackend,
        "kpm-incompatible" to DashboardIssue.KpmIncompatible("1.1.1", "1.2.0", "Fixture: the loaded vpnhide KPM is version 1.1.1"),
        "lsposed-missing" to DashboardIssue.LsposedNotInstalled,
        "lsposed-reboot" to DashboardIssue.LsposedNeedsReboot,
        "lsposed-disabled" to DashboardIssue.LsposedNotEnabled,
        "lsposed-scope-missing" to DashboardIssue.LsposedNoSystemScope,
        "targets-empty" to DashboardIssue.NoTargets,
        "ports-inactive" to DashboardIssue.PortsRulesInactive("Fixture: apply failed"),
        "selinux-permissive" to DashboardIssue.SelinuxPermissive,
    )

internal val agentUiScenarioCatalog: List<AgentUiScenarioSpec> =
    agentUiIssues.keys.map { AgentUiScenarioSpec(it, "Production dashboard issue and its help action: $it") } +
        listOf(
            AgentUiScenarioSpec("dashboard-loading", "Production dashboard loading skeleton"),
            AgentUiScenarioSpec("help", "Offline guide; destination accepts article-id#localized-anchor, or null for contents"),
        )

internal fun validateAgentUiStart(args: AgentUiStartArgs) {
    require(agentUiScenarioCatalog.any { it.id == args.scenario }) { "Unknown UI scenario: ${args.scenario}" }
    require(args.destination == null || args.scenario == "help") { "destination is only valid for the help scenario" }
    validateAgentUiDestination(args.destination)
}

internal fun validateAgentUiDestination(destination: String?) {
    if (destination == null) return
    require(destination.length in 1..512 && destination.none { it.isISOControl() }) { "Invalid help destination" }
    val article = destination.substringBefore('#')
    require(article.matches(Regex("[a-z0-9]+(?:-[a-z0-9]+)*"))) { "Use article-id#anchor, not a URL or path" }
}
