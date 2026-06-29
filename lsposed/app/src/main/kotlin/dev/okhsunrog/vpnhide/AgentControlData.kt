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
        "kernel", "kmod", "kpm" -> NativeHookFamily.Kernel
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

internal fun parseAgentPortPolicyResult(
    mode: String,
    preset: String?,
    rules: List<AgentPortRule>,
): Result<PortPolicy?> {
    val normalizedMode = mode.trim().lowercase()
    if (normalizedMode == "all") return Result.success(null)
    if (normalizedMode == PortPolicyMode.Preset.jsonName) {
        val presetId = preset ?: return Result.failure(IllegalArgumentException("Port preset is required"))
        return portPolicyForPreset(presetId)?.let(Result.Companion::success)
            ?: Result.failure(IllegalArgumentException("Unknown port preset: $presetId"))
    }
    val parsedRules =
        try {
            rules.map {
                PortRule(
                    protocol = PortProtocol.fromJson(it.protocol.trim().lowercase()),
                    start = it.start,
                    end = it.end,
                )
            }
        } catch (e: IllegalArgumentException) {
            return Result.failure(e)
        }
    if (parsedRules.isEmpty()) {
        return Result.failure(IllegalArgumentException("Custom port policy requires at least one rule"))
    }
    return Result.success(
        normalizePortPolicy(
            PortPolicy(
                mode = PortPolicyMode.Custom,
                preset = null,
                rules = parsedRules,
            ),
        ),
    )
}
