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
internal fun buildAppHelperStageCommand(asset: AppHelperAsset): String {
    val target = appHelperStagedPath(asset.digest)
    val temporary = "$target.tmp-${UUID.randomUUID()}"
    return "umask 077; if [ ! -f ${shellQuote(target)} ]; then " +
        "cp ${shellQuote(asset.local.absolutePath)} ${shellQuote(temporary)} && chmod 700 ${shellQuote(temporary)} && " +
        "{ ln ${shellQuote(temporary)} ${shellQuote(target)} 2>/dev/null || true; }; " +
        "rm -f ${shellQuote(temporary)}; fi; " +
        "[ \"\$(sha256sum ${shellQuote(target)} | cut -d ' ' -f 1)\" = '${asset.digest}' ]"
}
