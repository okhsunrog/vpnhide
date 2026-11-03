package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds

internal fun requirePackageName(packageName: String): String {
    val pkg = packageName.trim()
    require(pkg.isNotEmpty() && pkg.none(Char::isWhitespace)) { "Invalid package name: $packageName" }
    return pkg
}

internal fun resolveHookIds(
    hookIds: List<String>,
    entries: List<HookIds.Hook>,
): List<String> {
    val allowed = entries.mapTo(sortedSetOf()) { it.hookName }
    val unknown = hookIds.filterNot { it in allowed }
    require(unknown.isEmpty()) { "Unknown hook id(s): ${unknown.joinToString()}" }
    return hookIds.distinct()
}

internal fun parseNativeHookFamily(value: String): NativeHookFamily =
    when (value.trim().lowercase()) {
        "kernel" -> NativeHookFamily.Kernel
        "kmod" -> NativeHookFamily.Kmod
        "kpm" -> NativeHookFamily.Kpm
        "zygisk" -> NativeHookFamily.Zygisk
        else -> throw IllegalArgumentException("Unknown native hook family: $value")
    }

internal fun parseAgentPortPolicy(
    mode: String,
    preset: String?,
    rules: List<AgentPortRule>,
): PortPolicy? =
    parseAgentPortPolicyResult(mode, preset, rules).getOrElse { error ->
        throw IllegalArgumentException(error.message ?: "Invalid port policy")
    }
