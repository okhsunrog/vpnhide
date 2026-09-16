package dev.okhsunrog.vpnhide

import android.content.Context
import android.os.Build
import dev.okhsunrog.vpnhide.startup.StartupTrace
import java.io.File
import java.security.MessageDigest
import java.util.UUID

private const val MUTATION_DIRECTORY = "/data/adb/vpnhide/app-state"

/** Versioned executables avoid replacing a binary still supervising a previous app process. */
internal fun prepareRootMutationTransport(
    context: Context,
    runner: RootProcessRunner,
): RootMutationTransport? =
    try {
        val abi = Build.SUPPORTED_ABIS.first()
        val bytes = context.assets.open("bin/$abi/vhmutate").use { it.readBytes() }
        val digest = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
        val local = File(context.filesDir, "vhmutate-$digest")
        if (!local.exists()) {
            val temporary = File(context.filesDir, "vhmutate-${UUID.randomUUID()}.tmp")
            try {
                temporary.outputStream().use { it.write(bytes) }
                check(temporary.renameTo(local))
            } finally {
                temporary.delete()
            }
        }
        val executable = "$MUTATION_DIRECTORY/vhmutate-$digest"
        val command = buildRootMutationStageCommand(local.absolutePath, MUTATION_DIRECTORY, digest, UUID.randomUUID().toString())
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
