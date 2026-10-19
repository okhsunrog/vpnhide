package dev.okhsunrog.vpnhide.core

import android.content.Context
import android.os.Process

/**
 * Production peer-auth allowlist maintenance (step 7).
 *
 * After a one-shot su grant (or Magisk app grant), the APK writes its own uid into
 * [CorePaths.ALLOWED_UIDS] so the core's SO_PEERCRED check admits this install. On
 * reinstall / UID change, call [refresh] again — the daemon reads the file at start
 * (and env override); a core restart picks up the new uid.
 *
 * The write itself still needs root (persist dir is 0700 under /data/adb). Callers pass
 * a root shell runner; this type does not invoke su by itself.
 */
object CoreAllowlist {
    /**
     * Shell snippet that writes the current app uid into the allowlist file.
     * Execute under root (existing [dev.okhsunrog.vpnhide.RootProcessRunner] / agent).
     */
    fun writeSelfUidScript(context: Context): String {
        val uid = Process.myUid()
        val pkg = context.packageName
        val path = CorePaths.ALLOWED_UIDS
        val dir = CorePaths.PERSIST_DIR
        return """
            mkdir -p '$dir'
            chmod 700 '$dir' 2>/dev/null
            printf '%s\n' '$uid' > '$path'
            chmod 600 '$path' 2>/dev/null
            # Optional: record package for operators (core ignores non-numeric lines).
            printf '# %s\n' '$pkg' >> '$path'
            echo ok uid=$uid
        """.trimIndent()
    }
}
