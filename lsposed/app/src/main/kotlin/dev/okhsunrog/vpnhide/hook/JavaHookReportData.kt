package dev.okhsunrog.vpnhide.hook

/** Bounded install-time evidence only: no method arguments or live Binder payloads. */
internal class JavaHookReport {
    private val rows = linkedMapOf<String, String>()
    private val origins = linkedSetOf<String>()
    private var truncated = false

    @Synchronized
    fun record(
        key: String,
        value: String,
    ): Map<String, String> {
        if (value.length > MAX_VALUE || key.length > 192) truncated = true
        if (rows.size < MAX_ROWS || key in rows) {
            rows[key.take(192)] = value.replace('\n', ' ').replace('\r', ' ').take(MAX_VALUE)
        } else {
            truncated = true
        }
        return snapshot()
    }

    @Synchronized
    fun loader(description: String): Map<String, String> {
        // ClassLoader.toString is best-effort, not a promised ART reflection API.
        ORIGIN.findAll(description.take(8192)).take(32).forEach {
            if (origins.size < 16 && it.value.length <= 1024) origins += it.value else truncated = true
        }
        return snapshot()
    }

    @Synchronized
    fun snapshot(): Map<String, String> =
        buildMap {
            put("java_report_schema", "1")
            put("java_report_truncated", truncated.toString())
            rows.entries.forEachIndexed { index, entry -> put("java_evidence_$index", "${entry.key}: ${entry.value}") }
            origins.forEachIndexed { index, path -> put("java_origin_$index", path) }
        }

    companion object {
        private const val MAX_ROWS = 96
        private const val MAX_VALUE = 768
        private val ORIGIN = Regex("/(?:system|system_ext|product|vendor|apex)/[^\\s\\\"\\],;:]+\\.(?:jar|apk)")
    }
}

internal fun describeHookMethods(
    clazz: Class<*>,
    names: Set<String>,
): List<String> =
    generateSequence(clazz) { it.superclass }
        .take(4)
        .flatMap { owner ->
            owner.declaredMethods
                .asSequence()
                .filter { it.name in names }
                .map { method ->
                    "${method.declaringClass.name}.${method.name}(${method.parameterTypes.joinToString(
                        ",",
                    ) { it.name }}):${method.returnType.name}"
                }
        }.sorted()
        .take(32)
        .toList()
