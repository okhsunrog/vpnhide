package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.TargetsSnapshot

internal fun nativeHookFamilyFor(backend: NativeBackendId?): NativeHookFamily =
    when (backend) {
        NativeBackendId.Zygisk -> NativeHookFamily.Zygisk

        NativeBackendId.Kmod -> NativeHookFamily.Kmod

        // The in-tree backend owns the same kernel hooks as the .ko (including the
        // optional filesystem hook), so it shares the kmod config hook family.
        NativeBackendId.Builtin -> NativeHookFamily.Kmod

        NativeBackendId.Kpm -> NativeHookFamily.Kpm

        null -> NativeHookFamily.Kernel
    }

internal fun buildCanonicalConfig(
    debug: Boolean,
    // null = inherit the user's toggle intent from [existing] (or fall back to
    // [debug] on a first-write). Rebuild paths (Save / auto-hide / manual-hidden)
    // must NOT reset debugSwitch to debug — the two diverge during a capture, and
    // defaulting to debug would clobber the persisted intent.
    debugSwitch: Boolean? = null,
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
        debugSwitch = debugSwitch ?: existing?.debugSwitch ?: debug,
        apps = apps,
        settings = existing?.settings ?: CanonicalSettings(),
    )
}

internal fun buildCanonicalConfigFromTargetsSnapshot(
    snapshot: TargetsSnapshot,
    debug: Boolean = snapshot.canonicalConfig?.debug ?: false,
    debugSwitch: Boolean = snapshot.canonicalConfig?.debugSwitch ?: debug,
): CanonicalConfig =
    buildCanonicalConfig(
        debug = debug,
        debugSwitch = debugSwitch,
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
            // Never a ports target: the ports module would reject this app's own
            // loopback traffic, which cuts the agent bridge off, and no self-check
            // measures localhost hiding. Reachable only through the bridge or an
            // import; the picker does not list this app.
            ports = false,
            portPolicy = null,
        )
    if (updated == current && config.apps.containsKey(selfPkg)) return config
    return config.copy(apps = (config.apps + (selfPkg to updated)).toSortedMap())
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
        append("  \"debugSwitch\": ")
        append(config.debugSwitch)
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
        append("    \"optionalFeatures\": [")
        config.settings.optionalFeatures.toSortedSet().forEachIndexed { index, feature ->
            if (index != 0) append(", ")
            appendJsonString(feature)
        }
        append("],\n")
        append("    \"autoHideVpnServices\": ")
        append(config.settings.autoHideVpnServices)
        append(",\n")
        append("    \"autoHideVpnName\": ")
        append(config.settings.autoHideVpnName)
        append(",\n")
        append("    \"autoHideExcludedPackages\": [")
        config.settings.autoHideExcludedPackages.toSortedSet().forEachIndexed { index, pkg ->
            if (index != 0) append(", ")
            appendJsonString(pkg)
        }
        append("],\n")
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
