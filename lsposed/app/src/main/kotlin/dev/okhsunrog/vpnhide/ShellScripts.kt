package dev.okhsunrog.vpnhide

/**
 * The root-shell scripts, loaded from `src/main/resources/shell/` instead of
 * living inside Kotlin string literals.
 *
 * They used to be `"""…"""` blocks with every `$` written as `${'$'}`. That cost
 * us a shipped bug: an apostrophe inside a `'…'`-quoted block ended the quoting,
 * the whole batched command stopped parsing, and every forensic section vanished
 * from bug reports for a release (#306). As plain `.sh` files they are
 * syntax-highlighted, `shellcheck`-able in CI, and diff as shell rather than as
 * escaped Kotlin.
 *
 * Parameters are passed as leading variable assignments rather than by
 * interpolating into the script body — the values (paths, framing prefixes)
 * stay defined once in Kotlin, and the script stays a file you can lint and run.
 */
internal object ShellScripts {
    private val cache = HashMap<String, String>()

    /** Read a script from the app's Java resources. Cached: the scripts are
     *  immutable and the root snapshot runs on the startup path. */
    @Synchronized
    fun load(name: String): String =
        cache.getOrPut(name) {
            val path = "shell/$name"
            val stream =
                ShellScripts::class.java.classLoader?.getResourceAsStream(path)
                    ?: error("shell script resource missing: $path")
            stream.bufferedReader().use { it.readText() }
        }
}

/**
 * A `VAR='value'` prelude for [ShellScripts.load]'s inputs.
 *
 * Single-quoted so nothing in a value is re-interpreted by the shell; an
 * embedded quote is closed and re-opened the POSIX way (`'\''`). Every value we
 * pass is a compile-time constant today, so this is belt and braces — but the
 * next one might not be.
 */
internal fun shellVariables(vars: Map<String, String>): String =
    vars.entries.joinToString(separator = "\n", postfix = "\n") { (name, value) ->
        "$name='${value.replace("'", "'\\''")}'"
    }

/** A script preceded by the variables it reads. */
internal fun shellScriptWith(
    name: String,
    vars: Map<String, String>,
): String = shellVariables(vars) + ShellScripts.load(name)
