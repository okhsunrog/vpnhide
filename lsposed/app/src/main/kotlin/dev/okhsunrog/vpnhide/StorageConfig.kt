package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds
import org.json.JSONArray
import org.json.JSONObject

internal const val CANONICAL_CONFIG_FILE = "/data/system/vpnhide_config.json"
internal const val SUPERKEY_FILE = "/data/adb/vpnhide/superkey"

internal data class CanonicalConfig(
    val version: Int = 1,
    val debug: Boolean = false,
    val apps: Map<String, CanonicalApp> = emptyMap(),
    val settings: CanonicalSettings = CanonicalSettings(),
)

internal data class CanonicalSettings(
    val rememberSuperkey: Boolean = false,
    val autoHideVpnServices: Boolean = true,
    val autoHideVpnName: Boolean = false,
    val autoHiddenPackages: Set<String> = emptySet(),
)

internal data class CanonicalApp(
    val java: Boolean = false,
    val javaHooks: List<String>? = null,
    val native: NativeRole = NativeRole.Disabled,
    val appHiding: Boolean = false,
    val ports: Boolean = false,
    val portPolicy: PortPolicy? = null,
    val hidden: Boolean = false,
) {
    val hasAnyRole: Boolean
        get() = java || native.enabled || appHiding || ports || hidden
}

internal data class NativeRole(
    val enabled: Boolean,
    val overrides: NativeHookOverrides = NativeHookOverrides(),
) {
    companion object {
        val Disabled = NativeRole(enabled = false)
        val All = NativeRole(enabled = true)
    }
}

internal enum class NativeHookFamily {
    Kernel,
    Zygisk,
}

internal data class NativeHookOverrides(
    val kernel: List<String>? = null,
    val zygisk: List<String>? = null,
) {
    fun hooksFor(family: NativeHookFamily): List<String>? =
        when (family) {
            NativeHookFamily.Kernel -> kernel
            NativeHookFamily.Zygisk -> zygisk
        }

    fun withHooksFor(
        family: NativeHookFamily,
        hooks: List<String>?,
    ): NativeHookOverrides =
        when (family) {
            NativeHookFamily.Kernel -> copy(kernel = hooks)
            NativeHookFamily.Zygisk -> copy(zygisk = hooks)
        }

    val hasAnyOverride: Boolean
        get() = kernel != null || zygisk != null
}

private data class ParsedHookRole(
    val enabled: Boolean,
    val hooks: List<String>? = null,
)

internal val NativeKernelHookEntries: List<HookIds.Hook> =
    HookIds.Hook.entries.filter { hookBit(it) and HookIds.KERNEL_HOOK_MASK.toLong() != 0L }

internal val ZygiskNativeHookEntries: List<HookIds.Hook> =
    HookIds.Hook.entries.filter { hookBit(it) and HookIds.ZYGISK_HOOK_MASK.toLong() != 0L }

internal val NativeHookEntries: List<HookIds.Hook> = NativeKernelHookEntries

internal val LsposedJavaHookEntries: List<HookIds.Hook> =
    HookIds.Hook.entries.filter {
        hookBit(it) and HookIds.LSPOSED_HOOK_MASK.toLong() != 0L &&
            it != HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY
    }

internal fun nativeHookEntriesFor(family: NativeHookFamily): List<HookIds.Hook> =
    when (family) {
        NativeHookFamily.Kernel -> NativeKernelHookEntries
        NativeHookFamily.Zygisk -> ZygiskNativeHookEntries
    }

internal fun nativeHookFamilyFor(backend: NativeBackendId?): NativeHookFamily =
    when (backend) {
        NativeBackendId.Zygisk -> NativeHookFamily.Zygisk
        NativeBackendId.Kmod, NativeBackendId.Kpm, null -> NativeHookFamily.Kernel
    }

internal fun hookSelectionMask(
    enabled: Boolean,
    hooks: List<String>?,
    entries: List<HookIds.Hook>,
): Long {
    if (!enabled) return 0L
    val allMask = entries.fold(0L) { acc, hook -> acc or hookBit(hook) }
    if (hooks == null) return allMask
    val allowedByName = entries.associateBy { it.hookName }
    return hooks.fold(0L) { acc, name -> acc or (allowedByName[name]?.let(::hookBit) ?: 0L) }
}

private fun hookBit(hook: HookIds.Hook): Long = 1L shl hook.id

internal fun parseCanonicalConfig(raw: String): CanonicalConfig? {
    if (raw.isBlank()) return null
    val root = JSONObject(raw)
    val version = root.optInt("version", 1)
    if (version > 1) return null
    val appsJson = root.optJSONObject("apps")
    val apps =
        appsJson
            ?.keys()
            ?.asSequence()
            ?.sorted()
            ?.associateWith { pkg -> parseCanonicalApp(appsJson.optJSONObject(pkg)) }
            .orEmpty()
            .filterValues { it.hasAnyRole }
    val settingsJson = root.optJSONObject("settings")
    val defaultSettings = CanonicalSettings()
    return CanonicalConfig(
        version = version,
        debug = root.optBoolean("debug", false),
        apps = apps,
        settings =
            CanonicalSettings(
                rememberSuperkey = settingsJson?.optBoolean("rememberSuperkey", defaultSettings.rememberSuperkey) == true,
                autoHideVpnServices =
                    settingsJson?.optBoolean("autoHideVpnServices", defaultSettings.autoHideVpnServices)
                        ?: defaultSettings.autoHideVpnServices,
                autoHideVpnName =
                    settingsJson?.optBoolean("autoHideVpnName", defaultSettings.autoHideVpnName)
                        ?: defaultSettings.autoHideVpnName,
                autoHiddenPackages = parseStringSet(settingsJson?.optJSONArray("autoHiddenPackages")),
            ),
    )
}

private fun parseCanonicalApp(obj: JSONObject?): CanonicalApp {
    if (obj == null) return CanonicalApp()
    val java = parseHookRole(obj.opt("java"))
    val ports = obj.optBoolean("ports", false)
    return CanonicalApp(
        java = java.enabled,
        javaHooks = java.hooks,
        native = parseNativeRole(obj.opt("native")),
        appHiding = obj.optBoolean("appHiding", false),
        ports = ports,
        portPolicy = if (ports) parsePortPolicy(obj.optJSONObject("portPolicy")) else null,
        hidden = obj.optBoolean("hidden", false),
    )
}

private fun parsePortPolicy(obj: JSONObject?): PortPolicy? {
    if (obj == null) return null
    // Best-effort: a single malformed rule (e.g. an out-of-range port that trips
    // PortRule's require()) must not throw and unwind the WHOLE canonical config,
    // which would silently disable every hook. Drop the offending rule/policy and
    // treat the app as ports-disabled instead.
    return runCatching {
        val rulesJson = obj.optJSONArray("rules") ?: return@runCatching null
        val rules =
            (0 until rulesJson.length()).mapNotNull { idx ->
                val ruleObj = rulesJson.optJSONObject(idx) ?: return@mapNotNull null
                runCatching { parsePortRule(ruleObj) }.getOrNull()
            }
        normalizePortPolicy(
            PortPolicy(
                mode = PortPolicyMode.fromJson(obj.optString("mode", PortPolicyMode.Custom.jsonName)),
                preset =
                    if (obj.has("preset") && !obj.isNull("preset")) {
                        obj.optString("preset").takeIf { it.isNotBlank() }
                    } else {
                        null
                    },
                rules = rules,
            ),
        )
    }.getOrNull()
}

private fun parsePortRule(obj: JSONObject): PortRule {
    val start = obj.optInt("start", -1)
    val end = if (obj.has("end")) obj.optInt("end", start) else start
    return PortRule(
        protocol = PortProtocol.fromJson(obj.optString("protocol", PortProtocol.Both.jsonName)),
        start = start,
        end = end,
    )
}

private fun parseNativeRole(value: Any?): NativeRole =
    when (value) {
        is JSONObject -> {
            parseNativeRoleObject(value)
        }

        else -> {
            parseHookRole(value).let { role ->
                if (role.enabled) {
                    NativeRole(
                        enabled = true,
                        overrides = NativeHookOverrides(kernel = role.hooks),
                    )
                } else {
                    NativeRole.Disabled
                }
            }
        }
    }

private fun parseNativeRoleObject(obj: JSONObject): NativeRole {
    val enabled = obj.optBoolean("enabled", obj.has("kernel") || obj.has("zygisk"))
    if (!enabled) return NativeRole.Disabled
    return NativeRole(
        enabled = true,
        overrides =
            NativeHookOverrides(
                kernel = parseOptionalStringList(obj, "kernel"),
                zygisk = parseOptionalStringList(obj, "zygisk"),
            ),
    )
}

private fun parseHookRole(value: Any?): ParsedHookRole =
    when (value) {
        is Boolean -> {
            if (value) ParsedHookRole(enabled = true) else ParsedHookRole(enabled = false)
        }

        is JSONArray -> {
            val hooks =
                (0 until value.length())
                    .mapNotNull { idx -> value.optString(idx).takeIf { it.isNotBlank() } }
            if (hooks.isEmpty()) ParsedHookRole(enabled = false) else ParsedHookRole(enabled = true, hooks = hooks)
        }

        else -> {
            ParsedHookRole(enabled = false)
        }
    }

private fun parseStringSet(value: JSONArray?): Set<String> =
    (0 until (value?.length() ?: 0))
        .mapNotNull { idx -> value?.optString(idx)?.takeIf { it.isNotBlank() } }
        .toSortedSet()

private fun parseOptionalStringList(
    obj: JSONObject,
    key: String,
): List<String>? {
    if (!obj.has(key) || obj.isNull(key)) return null
    val array = obj.optJSONArray(key) ?: return null
    return (0 until array.length())
        .mapNotNull { idx -> array.optString(idx).takeIf { it.isNotBlank() } }
}

internal fun buildCanonicalConfig(
    debug: Boolean,
    javaPkgs: Collection<String>,
    nativePkgs: Collection<String>,
    hiddenPkgs: Collection<String>,
    observerPkgs: Collection<String>,
    portsPkgs: Collection<String>,
    existing: CanonicalConfig? = null,
): CanonicalConfig {
    val java = javaPkgs.toSet()
    val native = nativePkgs.toSet()
    val hidden = hiddenPkgs.toSet()
    val observers = observerPkgs.toSet()
    val ports = portsPkgs.toSet()
    val packages = (java + native + hidden + observers + ports).sorted()
    val apps =
        packages
            .associateWith { pkg ->
                val previous = existing?.apps?.get(pkg)
                val previousNative = previous?.native
                CanonicalApp(
                    java = pkg in java,
                    javaHooks = previous?.javaHooks?.takeIf { pkg in java && previous.java },
                    native = if (pkg in native) previousNative?.takeIf { it.enabled } ?: NativeRole.All else NativeRole.Disabled,
                    appHiding = pkg in observers,
                    ports = pkg in ports,
                    portPolicy = previous?.portPolicy?.takeIf { pkg in ports && previous.ports },
                    hidden = pkg in hidden,
                )
            }.filterValues { it.hasAnyRole }
    return CanonicalConfig(
        version = 1,
        debug = debug,
        apps = apps,
        settings = existing?.settings ?: CanonicalSettings(),
    )
}

internal fun buildCanonicalConfigFromTargetsSnapshot(
    snapshot: TargetsSnapshot,
    debug: Boolean = snapshot.canonicalConfig?.debug ?: false,
): CanonicalConfig =
    buildCanonicalConfig(
        debug = debug,
        javaPkgs = snapshot.lsposedTargets,
        nativePkgs = snapshot.nativeTargets,
        hiddenPkgs = snapshot.hiddenPkgs,
        observerPkgs = snapshot.observerNames,
        portsPkgs = snapshot.portsObservers,
        existing = snapshot.canonicalConfig,
    )

internal fun canonicalConfigWithSelfTarget(
    config: CanonicalConfig,
    selfPkg: String,
): CanonicalConfig {
    val current = config.apps[selfPkg] ?: CanonicalApp()
    val updated =
        current.copy(
            java = true,
            javaHooks = null,
            native = NativeRole.All,
            hidden = true,
        )
    if (updated == current && config.apps.containsKey(selfPkg)) return config
    return config.copy(apps = (config.apps + (selfPkg to updated)).toSortedMap())
}

internal fun parseImportedCanonicalConfig(
    raw: String,
    selfPkg: String,
): CanonicalConfig? =
    try {
        parseCanonicalConfig(raw)?.let { canonicalConfigWithSelfTarget(it, selfPkg) }
    } catch (_: Throwable) {
        null
    }

internal fun canonicalConfigJson(config: CanonicalConfig): String =
    buildString {
        append("{\n")
        append("  \"version\": ")
        append(config.version)
        append(",\n")
        append("  \"debug\": ")
        append(config.debug)
        append(",\n")
        append("  \"apps\": {")
        val apps = config.apps.toSortedMap().filterValues { it.hasAnyRole }
        if (apps.isNotEmpty()) append('\n')
        apps.entries.forEachIndexed { index, (pkg, app) ->
            append("    ")
            appendJsonString(pkg)
            append(": ")
            appendCanonicalApp(app)
            if (index != apps.size - 1) append(',')
            append('\n')
        }
        append(if (apps.isEmpty()) "},\n" else "  },\n")
        append("  \"settings\": {\n")
        append("    \"rememberSuperkey\": ")
        append(config.settings.rememberSuperkey)
        append(",\n")
        append("    \"autoHideVpnServices\": ")
        append(config.settings.autoHideVpnServices)
        append(",\n")
        append("    \"autoHideVpnName\": ")
        append(config.settings.autoHideVpnName)
        append(",\n")
        append("    \"autoHiddenPackages\": [")
        config.settings.autoHiddenPackages.toSortedSet().forEachIndexed { index, pkg ->
            if (index != 0) append(", ")
            appendJsonString(pkg)
        }
        append("]\n")
        append("  }\n")
        append("}\n")
    }

private fun StringBuilder.appendCanonicalApp(app: CanonicalApp) {
    append("{ ")
    append("\"java\": ")
    appendHookRole(app.java, app.javaHooks)
    append(", \"native\": ")
    appendNativeRole(app.native)
    append(", \"appHiding\": ")
    append(app.appHiding)
    append(", \"ports\": ")
    append(app.ports)
    if (app.ports) {
        normalizePortPolicy(app.portPolicy)?.let { policy ->
            append(", \"portPolicy\": ")
            appendPortPolicy(policy)
        }
    }
    if (app.hidden) {
        append(", \"hidden\": true")
    }
    append(" }")
}

private fun StringBuilder.appendPortPolicy(policy: PortPolicy) {
    append("{ \"mode\": ")
    appendJsonString(policy.mode.jsonName)
    policy.preset?.takeIf { it.isNotBlank() }?.let { preset ->
        append(", \"preset\": ")
        appendJsonString(preset)
    }
    append(", \"rules\": [")
    normalizedPortRules(policy.rules).forEachIndexed { index, rule ->
        if (index != 0) append(", ")
        appendPortRule(rule)
    }
    append("] }")
}

private fun StringBuilder.appendPortRule(rule: PortRule) {
    append("{ \"protocol\": ")
    appendJsonString(rule.protocol.jsonName)
    append(", \"start\": ")
    append(rule.start)
    if (rule.end != rule.start) {
        append(", \"end\": ")
        append(rule.end)
    }
    append(" }")
}

private fun StringBuilder.appendNativeRole(native: NativeRole) {
    when {
        !native.enabled -> {
            append("false")
        }

        !native.overrides.hasAnyOverride -> {
            append("true")
        }

        else -> {
            append("{ \"enabled\": true")
            native.overrides.kernel?.let { hooks ->
                append(", \"kernel\": ")
                appendStringArray(hooks)
            }
            native.overrides.zygisk?.let { hooks ->
                append(", \"zygisk\": ")
                appendStringArray(hooks)
            }
            append(" }")
        }
    }
}

private fun StringBuilder.appendHookRole(
    enabled: Boolean,
    hooks: List<String>?,
) {
    when {
        !enabled -> append("false")
        hooks == null -> append("true")
        else -> appendStringArray(hooks)
    }
}

private fun StringBuilder.appendStringArray(values: List<String>) {
    append('[')
    values.forEachIndexed { index, value ->
        if (index != 0) append(", ")
        appendJsonString(value)
    }
    append(']')
}

private fun StringBuilder.appendJsonString(value: String) {
    append('"')
    value.forEach { ch ->
        when (ch) {
            '\\' -> {
                append("\\\\")
            }

            '"' -> {
                append("\\\"")
            }

            '\b' -> {
                append("\\b")
            }

            '\u000C' -> {
                append("\\f")
            }

            '\n' -> {
                append("\\n")
            }

            '\r' -> {
                append("\\r")
            }

            '\t' -> {
                append("\\t")
            }

            else -> {
                if (ch < ' ') {
                    append("\\u")
                    append(ch.code.toString(16).padStart(4, '0'))
                } else {
                    append(ch)
                }
            }
        }
    }
    append('"')
}

internal fun buildCanonicalConfigWriteCommand(config: CanonicalConfig): String =
    buildAtomicSystemDataRawWriteCommand(CANONICAL_CONFIG_FILE, canonicalConfigJson(config), "640")

internal fun buildSuperkeyWriteCommand(superkey: String): String =
    listOf(
        "mkdir -p /data/adb/vpnhide",
        buildAtomicRootOnlyRawWriteCommand(SUPERKEY_FILE, superkey.trim() + "\n"),
    ).joinToString(" && ")

internal fun buildSuperkeyClearCommand(): String = "rm -f $SUPERKEY_FILE"

internal fun buildLegacyConfigCleanupCommand(): String =
    listOf(
        "rm -f " +
            listOf(
                KMOD_TARGETS,
                KPM_TARGETS,
                ZYGISK_TARGETS,
                LSPOSED_TARGETS,
                PORTS_OBSERVERS_FILE,
                SS_HIDDEN_PKGS_FILE,
                SS_OBSERVER_UIDS_FILE,
                "/data/system/vpnhide_uids.txt",
                LEGACY_HOOK_STATUS_FILE,
            ).joinToString(" "),
        "rmdir /data/adb/vpnhide_zygisk /data/adb/vpnhide_lsposed /data/adb/vpnhide_ports 2>/dev/null || true",
    ).joinToString(" ; ")
