package dev.okhsunrog.vpnhide.hook

import dev.okhsunrog.vpnhide.LsposedStats

/** Reuses the existing LSPosed state writer and boot identity. Never writes on hook calls. */
internal object JavaHookDiagnostics {
    private val report = JavaHookReport()

    fun record(
        key: String,
        value: String,
    ) {
        runCatching { LsposedStats.setJavaDiagnostics(report.record(key, value)) }
    }

    fun inspect(
        clazz: Class<*>,
        methods: Set<String>,
    ) {
        runCatching {
            val loader = clazz.classLoader
            record("class ${clazz.name}", "super=${clazz.superclass?.name} loader=${loader?.javaClass?.name ?: "boot"}")
            generateSequence(loader) { it.parent }.take(4).forEach {
                LsposedStats.setJavaDiagnostics(report.loader(it.toString()))
            }
            val signatures = describeHookMethods(clazz, methods)
            record("methods ${clazz.name} ${methods.sorted()}", signatures.joinToString(" | ").ifEmpty { "no matching methods" })
            signatures.forEachIndexed { index, signature -> record("signature ${clazz.name} ${methods.sorted()} $index", signature) }
        }.onFailure { record("inspection ${clazz.name}", "${it.javaClass.name}: ${it.message}") }
    }
}
