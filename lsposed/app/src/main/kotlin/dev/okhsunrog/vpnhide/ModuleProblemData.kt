package dev.okhsunrog.vpnhide

/** Cached diagnoses contain facts only; resolve strings with the current presentation locale. */
internal sealed interface ModuleProblem {
    val reason: ModuleBrokenReason?
    val downloadArtifact: String? get() = null

    data class Integrity(
        val problem: ModuleIntegrityProblem,
    ) : ModuleProblem {
        override val reason get() = problem.reason
    }
}

internal fun interface ModuleProblemStrings {
    fun getString(
        id: Int,
        vararg args: Any,
    ): String
}

internal fun ModuleProblem.renderText(strings: ModuleProblemStrings): String =
    when (this) {
        is ModuleProblem.Integrity -> renderModuleIntegrityProblem(problem, strings)
    }

private fun renderModuleIntegrityProblem(
    problem: ModuleIntegrityProblem,
    res: ModuleProblemStrings,
): String {
    val moduleName =
        when (problem.kind) {
            FlashableModuleKind.Kmod -> "kmod"
            FlashableModuleKind.Builtin -> "Built-in"
            FlashableModuleKind.Kpm -> "KPM"
            FlashableModuleKind.Zygisk -> "Zygisk"
            FlashableModuleKind.Core -> "Core"
        }
    return res.getString(R.string.dashboard_issue_module_artifact_missing, moduleName, problem.artifactPath)
}
