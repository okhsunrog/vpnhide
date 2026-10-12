package dev.okhsunrog.vpnhide.diagnostics

/** Resolve measured identities at render time; a language change must not rerun probes. */
internal fun diagnosticLabelResource(id: String): Int? =
    NATIVE_CHECKS.firstOrNull { it.id == id }?.labelRes
        ?: (NATIVE_EXTRA_CHECKS + CORE_JAVA_CHECKS + EXTRA_JAVA_CHECKS).firstOrNull { it.id == id }?.labelRes
