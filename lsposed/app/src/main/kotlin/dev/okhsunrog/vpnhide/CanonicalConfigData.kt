package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds
import org.json.JSONArray
import org.json.JSONObject

internal const val CANONICAL_CONFIG_FILE = "/data/system/vpnhide_config.json"
internal const val SUPERKEY_FILE = "/data/adb/vpnhide/superkey"
internal val OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS = HookIds.Hook.FILESYSTEM_IFACE_PATHS.hookName

internal data class CanonicalConfig(
    val version: Int = 1,
    val debug: Boolean = false,
    val debugSwitch: Boolean = false,
    val apps: Map<String, CanonicalApp> = emptyMap(),
    val settings: CanonicalSettings = CanonicalSettings(),
)

internal data class CanonicalSettings(
    val rememberSuperkey: Boolean = false,
    val optionalFeatures: Set<String> = emptySet(),
    val autoHideVpnServices: Boolean = true,
    val autoHideVpnName: Boolean = false,
    val autoHideExcludedPackages: Set<String> = emptySet(),
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
    Kmod,
    Kpm,
    Zygisk,
}

internal data class NativeHookOverrides(
    val kernel: List<String>? = null,
    val zygisk: List<String>? = null,
) {
    fun hooksFor(family: NativeHookFamily): List<String>? =
        when (family) {
            NativeHookFamily.Kernel, NativeHookFamily.Kmod, NativeHookFamily.Kpm -> kernel
            NativeHookFamily.Zygisk -> zygisk
        }

    fun withHooksFor(
        family: NativeHookFamily,
        hooks: List<String>?,
    ): NativeHookOverrides =
        when (family) {
            NativeHookFamily.Kernel, NativeHookFamily.Kmod, NativeHookFamily.Kpm -> copy(kernel = hooks)
            NativeHookFamily.Zygisk -> copy(zygisk = hooks)
        }

    val hasAnyOverride: Boolean
        get() = kernel != null || zygisk != null
}

private data class ParsedHookRole(
    val enabled: Boolean,
    val hooks: List<String>? = null,
)

internal val NativeKernelHookEntries: List<HookIds.Hook> = KERNEL_HOOKS.toList()

internal val NativeKmodHookEntries: List<HookIds.Hook> = (KERNEL_HOOKS + KMOD_HOOKS).toList()

internal val ZygiskNativeHookEntries: List<HookIds.Hook> = ZYGISK_HOOKS.toList()

internal val LsposedJavaHookEntries: List<HookIds.Hook> =
    LSPOSED_HOOKS.filter { it != HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY }

internal fun nativeHookEntriesFor(family: NativeHookFamily): List<HookIds.Hook> =
    when (family) {
        NativeHookFamily.Kernel -> NativeKernelHookEntries
        NativeHookFamily.Kmod -> NativeKmodHookEntries
        NativeHookFamily.Kpm -> NativeKernelHookEntries
        NativeHookFamily.Zygisk -> ZygiskNativeHookEntries
    }

internal fun hookSelectionMask(
    enabled: Boolean,
    hooks: List<String>?,
    entries: List<HookIds.Hook>,
): Long {
    if (!enabled) return 0L
    val allMask = entries.toHookMask()
    if (hooks == null) return allMask
    val allowedByName = entries.associateBy { it.hookName }
    return hooks.fold(0L) { acc, name -> acc or (allowedByName[name]?.bit ?: 0L) }
}

internal fun parseCanonicalConfig(raw: String): CanonicalConfig? {
    if (raw.isBlank()) return null
    val root = JSONObject(raw)
    val version = root.optInt("version", 1)
    if (version > 1) return null
    val debug = root.optBoolean("debug", false)
    val debugSwitch = root.optBoolean("debugSwitch", debug)
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
        debug = debug,
        debugSwitch = debugSwitch,
        apps = apps,
        settings =
            CanonicalSettings(
                rememberSuperkey = settingsJson?.optBoolean("rememberSuperkey", defaultSettings.rememberSuperkey) == true,
                optionalFeatures = parseStringSet(settingsJson?.optJSONArray("optionalFeatures")),
                autoHideVpnServices =
                    settingsJson?.optBoolean("autoHideVpnServices", defaultSettings.autoHideVpnServices)
                        ?: defaultSettings.autoHideVpnServices,
                autoHideVpnName =
                    settingsJson?.optBoolean("autoHideVpnName", defaultSettings.autoHideVpnName)
                        ?: defaultSettings.autoHideVpnName,
                autoHideExcludedPackages = parseStringSet(settingsJson?.optJSONArray("autoHideExcludedPackages")),
                autoHiddenPackages = parseStringSet(settingsJson?.optJSONArray("autoHiddenPackages")),
            ),
    )
}

private fun parseCanonicalApp(obj: JSONObject?): CanonicalApp {
    if (obj == null) return CanonicalApp()
    val java = parseHookRole(obj.opt("java"))
    val portsRequested = obj.optBoolean("ports", false)
    val portPolicyPresent = obj.has("portPolicy") && !obj.isNull("portPolicy")
    val portPolicy =
        if (portsRequested && portPolicyPresent) {
            obj.optJSONObject("portPolicy")?.let(::parsePortPolicy)
        } else {
            null
        }
    // A missing policy intentionally retains the legacy "all ports" meaning.
    // A present but malformed policy must instead disable the ports role: if it
    // were serialized as `ports=true` without a policy, the activator would
    // broaden the requested ranges into an all-ports block.
    val ports = portsRequested && (!portPolicyPresent || portPolicy != null)
    return CanonicalApp(
        java = java.enabled,
        javaHooks = java.hooks,
        native = parseNativeRole(obj.opt("native")),
        appHiding = obj.optBoolean("appHiding", false),
        ports = ports,
        portPolicy = portPolicy,
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
            (0 until rulesJson.length()).map { idx ->
                val ruleObj = requireNotNull(rulesJson.optJSONObject(idx)) { "Port rule must be an object" }
                parsePortRule(ruleObj)
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
    // A native object with no explicit `enabled` defaults to enabled, matching
    // the Rust activator's serde default (`NativeSelectionDetail.enabled = true`),
    // which is the authoritative projector. The old `has("kernel")||has("zygisk")`
    // default disagreed only for a bare `{}` object — display-only drift, aligned
    // here so the app shows what the activator would apply.
    val enabled = obj.optBoolean("enabled", true)
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
