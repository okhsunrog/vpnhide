package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.startup.StartupTrace

private const val MUTATION_DIRECTORY = "/data/adb/vpnhide/app-state"

/** Versioned executables avoid replacing a binary still supervising a previous app process. */
internal fun prepareRootMutationTransport(
    context: Context,
    runner: RootProcessRunner,
): RootMutationTransport? =
    try {
        val asset = extractAppHelper(context) ?: return null
        val executable = "$MUTATION_DIRECTORY/vhhelper-${asset.digest}"
        val command =
            buildRootMutationStageCommand(
                asset.local.absolutePath,
                MUTATION_DIRECTORY,
                asset.digest,
                java.util.UUID
                    .randomUUID()
                    .toString(),
            )
        when (runner.run(listOf("su", "-c", command))) {
            is RootProcessResult.Completed -> {
                StartupTrace.mark("config_transport_staged")
                RootMutationTransport(executable, "$MUTATION_DIRECTORY/lane", runner = runner)
            }

            RootProcessResult.Uncertain -> {
                null
            }
        }
    } catch (_: Exception) {
        null
    }
