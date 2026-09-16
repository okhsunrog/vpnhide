package dev.okhsunrog.vpnhide

import android.content.Context
import android.os.Build
import dev.okhsunrog.vpnhide.startup.StartupTrace
import java.io.File
import java.security.MessageDigest
import java.util.UUID

internal data class AppHelperAsset(
    val local: File,
    val digest: String,
)

/**
 * Extract the one app helper as an immutable, content-addressed file. A new APK
 * never overwrites an executable that an older app process may still be using.
 */
internal fun extractAppHelper(context: Context): AppHelperAsset? =
    runCatching {
        val abi = Build.SUPPORTED_ABIS.firstOrNull() ?: "arm64-v8a"
        val bytes = context.assets.open("bin/$abi/vhhelper").use { it.readBytes() }
        val digest = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
        val local = File(context.filesDir, "vhhelper-$digest")
        if (!local.exists()) {
            val temporary = File(context.filesDir, "vhhelper-${UUID.randomUUID()}.tmp")
            try {
                temporary.outputStream().use { it.write(bytes) }
                check(temporary.renameTo(local) || local.isFile)
            } finally {
                temporary.delete()
            }
        }
        StartupTrace.mark("app_helper_extracted")
        AppHelperAsset(local, digest)
    }.getOrNull()

internal fun appHelperStagedPath(digest: String): String {
    require(digest.matches(Regex("[0-9a-f]{64}")))
    return "/data/local/tmp/vpnhide-vhhelper-$digest"
}

/** Stage once by inode and verify the digest; an existing inode is never replaced. */
internal fun buildAppHelperStageCommand(asset: AppHelperAsset): String = buildAppHelperStageCommand(asset.local.absolutePath, asset.digest)

internal fun buildAppHelperStageCommand(
    source: String,
    digest: String,
): String {
    val target = appHelperStagedPath(digest)
    return buildContentAddressedExecutableStageCommand(
        source = source,
        target = target,
        digest = digest,
        temporary = "$target.tmp-${UUID.randomUUID()}",
    )
}

/**
 * Shared staging algorithm for every helper destination. The destination path
 * may differ for SELinux reasons, while publication and digest verification do not.
 */
internal fun buildContentAddressedExecutableStageCommand(
    source: String,
    target: String,
    digest: String,
    temporary: String,
    directory: String? = null,
): String {
    require(digest.matches(Regex("[0-9a-f]{64}")))
    val prepareDirectory =
        directory
            ?.let {
                "mkdir -p ${shellQuote(it)} && chmod 700 ${shellQuote(it)} && "
            }.orEmpty()
    val quotedTemporary = shellQuote(temporary)
    val quotedTarget = shellQuote(target)
    return "umask 077; $prepareDirectory" +
        "rm -f $quotedTemporary && (" +
        "if [ ! -f $quotedTarget ]; then " +
        "cp ${shellQuote(source)} $quotedTemporary && chmod 700 $quotedTemporary && " +
        "{ ln $quotedTemporary $quotedTarget 2>/dev/null || [ -f $quotedTarget ]; }; " +
        "VPNHIDE_STAGE_RC=\$?; rm -f $quotedTemporary; " +
        "[ \"\$VPNHIDE_STAGE_RC\" -eq 0 ] || exit \"\$VPNHIDE_STAGE_RC\"; fi; " +
        "[ -f $quotedTarget ] && " +
        "[ \"\$(sha256sum $quotedTarget | cut -d ' ' -f 1)\" = '$digest' ])"
}
