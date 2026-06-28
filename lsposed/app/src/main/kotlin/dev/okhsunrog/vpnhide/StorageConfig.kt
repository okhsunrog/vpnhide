package dev.okhsunrog.vpnhide

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
)

internal data class CanonicalApp(
    val java: Boolean = false,
    val native: NativeRole = NativeRole.Disabled,
    val appHiding: Boolean = false,
    val ports: Boolean = false,
    val hidden: Boolean = false,
) {
    val hasAnyRole: Boolean
        get() = java || native.enabled || appHiding || ports || hidden
}

internal data class NativeRole(
    val enabled: Boolean,
    val hooks: List<String>? = null,
) {
    companion object {
        val Disabled = NativeRole(enabled = false)
        val All = NativeRole(enabled = true)
    }
}

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
    return CanonicalConfig(
        version = version,
        debug = root.optBoolean("debug", false),
        apps = apps,
        settings = CanonicalSettings(rememberSuperkey = settingsJson?.optBoolean("rememberSuperkey", false) == true),
    )
}

private fun parseCanonicalApp(obj: JSONObject?): CanonicalApp {
    if (obj == null) return CanonicalApp()
    return CanonicalApp(
        java = obj.optBoolean("java", false),
        native = parseNativeRole(obj.opt("native")),
        appHiding = obj.optBoolean("appHiding", false),
        ports = obj.optBoolean("ports", false),
        hidden = obj.optBoolean("hidden", false),
    )
}

private fun parseNativeRole(value: Any?): NativeRole =
    when (value) {
        is Boolean -> {
            if (value) NativeRole.All else NativeRole.Disabled
        }

        is JSONArray -> {
            val hooks =
                (0 until value.length())
                    .mapNotNull { idx -> value.optString(idx).takeIf { it.isNotBlank() } }
            if (hooks.isEmpty()) NativeRole.Disabled else NativeRole(enabled = true, hooks = hooks)
        }

        else -> {
            NativeRole.Disabled
        }
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
                val previousNative = existing?.apps?.get(pkg)?.native
                CanonicalApp(
                    java = pkg in java,
                    native = if (pkg in native) previousNative?.takeIf { it.enabled } ?: NativeRole.All else NativeRole.Disabled,
                    appHiding = pkg in observers,
                    ports = pkg in ports,
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
            native = current.native.takeIf { it.enabled } ?: NativeRole.All,
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
        append('\n')
        append("  }\n")
        append("}\n")
    }

private fun StringBuilder.appendCanonicalApp(app: CanonicalApp) {
    append("{ ")
    append("\"java\": ")
    append(app.java)
    append(", \"native\": ")
    appendNativeRole(app.native)
    append(", \"appHiding\": ")
    append(app.appHiding)
    append(", \"ports\": ")
    append(app.ports)
    if (app.hidden) {
        append(", \"hidden\": true")
    }
    append(" }")
}

private fun StringBuilder.appendNativeRole(native: NativeRole) {
    when {
        !native.enabled -> {
            append("false")
        }

        native.hooks == null -> {
            append("true")
        }

        else -> {
            append('[')
            native.hooks.forEachIndexed { index, hook ->
                if (index != 0) append(", ")
                appendJsonString(hook)
            }
            append(']')
        }
    }
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
                "/data/system/vpnhide_debug_logging",
                LEGACY_HOOK_STATUS_FILE,
            ).joinToString(" "),
        "rmdir /data/adb/vpnhide_zygisk /data/adb/vpnhide_lsposed /data/adb/vpnhide_ports 2>/dev/null || true",
    ).joinToString(" ; ")
