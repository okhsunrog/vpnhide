package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.KpmProblemKind

/** Cached diagnoses contain facts only; resolve strings with the current presentation locale. */
internal sealed interface ModuleProblem {
    val reason: ModuleBrokenReason?
    val downloadArtifact: String? get() = null

    data class Kmod(
        val kind: KmodProblemKind,
    ) : ModuleProblem {
        override val reason get() = kind.reason
        override val downloadArtifact get() =
            when (val value = kind) {
                is KmodProblemKind.UnsupportedKernel -> value.recommendedArtifact
                is KmodProblemKind.WrongVariant -> value.recommendedArtifact
                is KmodProblemKind.UnknownVariant -> value.recommendedArtifact
                is KmodProblemKind.AmbiguousLoadFailed -> value.tryArtifact.takeIf { it.endsWith(".zip") }
                else -> null
            }
    }

    data class Kpm(
        val kind: KpmProblemKind,
    ) : ModuleProblem {
        override val reason get() = kind.reason
    }

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
        is ModuleProblem.Kmod -> renderKmodProblem(kind, strings)
        is ModuleProblem.Kpm -> renderKpmProblem(kind, strings)
        is ModuleProblem.Integrity -> renderModuleIntegrityProblem(problem, strings)
    }

private fun renderKmodProblem(
    kind: KmodProblemKind,
    res: ModuleProblemStrings,
): String =
    when (kind) {
        KmodProblemKind.KprobesMissing -> {
            res.getString(R.string.dashboard_issue_kprobes_missing)
        }

        KmodProblemKind.SignatureEnforced -> {
            res.getString(R.string.dashboard_issue_kmod_signature_enforced)
        }

        is KmodProblemKind.UnsupportedKernel -> {
            res.getString(R.string.dashboard_issue_kmod_not_supported_kernel, kind.unameR, kind.recommendedArtifact)
        }

        is KmodProblemKind.WrongVariant -> {
            res.getString(
                R.string.dashboard_issue_kmod_wrong_variant,
                kind.installedVariant,
                kind.recommendedKmi,
                kind.recommendedArtifact,
            )
        }

        is KmodProblemKind.UnknownVariant -> {
            res.getString(R.string.dashboard_issue_kmod_unknown_variant, kind.recommendedArtifact)
        }

        is KmodProblemKind.AmbiguousLoadFailed -> {
            res.getString(
                R.string.dashboard_issue_kmod_ambiguous_try_alternative,
                kind.installedVariant,
                kind.tryArtifact,
            )
        }

        is KmodProblemKind.LoadFailed -> {
            res.getString(R.string.dashboard_issue_kmod_load_failed, kind.insmodStderr)
        }
    }

private fun renderKpmProblem(
    kind: KpmProblemKind,
    res: ModuleProblemStrings,
): String =
    when (kind) {
        is KpmProblemKind.UnsupportedKernel -> {
            res.getString(R.string.dashboard_issue_kpm_unsupported_kernel, kind.unameR)
        }

        is KpmProblemKind.NoKernelPatchRuntime -> {
            res.getString(R.string.dashboard_issue_kpm_needs_runtime)
        }

        is KpmProblemKind.NeedsSuperkey -> {
            res.getString(R.string.dashboard_issue_kpm_needs_superkey)
        }

        is KpmProblemKind.LoadFailed -> {
            res.getString(R.string.dashboard_issue_kpm_load_failed, kind.detail)
        }
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
            FlashableModuleKind.Ports -> "Ports"
        }
    val message =
        when (problem.kind to problem.activatorState) {
            FlashableModuleKind.Kpm to ActivatorState.Missing -> {
                R.string.dashboard_issue_kpm_bundle_activator_missing
            }

            FlashableModuleKind.Kpm to ActivatorState.NotExecutable -> {
                R.string.dashboard_issue_kpm_bundle_activator_not_executable
            }

            else -> {
                when (problem.activatorState) {
                    ActivatorState.Missing -> {
                        R.string.dashboard_issue_module_activator_missing
                    }

                    ActivatorState.NotExecutable -> {
                        R.string.dashboard_issue_module_activator_not_executable
                    }

                    ActivatorState.Executable, ActivatorState.Unknown -> {
                        error("healthy or unknown activator is not an integrity problem")
                    }
                }
            }
        }
    val text =
        if (problem.kind == FlashableModuleKind.Kpm) {
            res.getString(message, problem.activatorPath)
        } else {
            res.getString(message, moduleName, problem.activatorPath)
        }
    return text
}
