package dev.okhsunrog.vpnhide

internal enum class CanonicalToggle(
    val path: List<String>,
) {
    Debug(listOf("debug")),
    DebugSwitch(listOf("debugSwitch")),
    RememberSuperkey(listOf("settings", "rememberSuperkey")),
    AutoHideServices(listOf("settings", "autoHideVpnServices")),
    AutoHideName(listOf("settings", "autoHideVpnName")),
}

internal enum class CanonicalAppField { Java, Native, AppHiding, Ports, Hidden }

internal enum class CanonicalSetField { OptionalFeatures, AutoHideExcludedPackages, AutoHiddenPackages }

internal sealed interface CanonicalEdit {
    data class Toggle(
        val field: CanonicalToggle,
        val enabled: Boolean,
    ) : CanonicalEdit

    data class SettingSet(
        val field: CanonicalSetField,
        val values: Set<String>,
    ) : CanonicalEdit

    data class App(
        val pkg: String,
        val field: CanonicalAppField,
        val value: CanonicalApp,
    ) : CanonicalEdit

    data class RemoveApp(
        val pkg: String,
    ) : CanonicalEdit

    data class Replace(
        val config: CanonicalConfig,
    ) : CanonicalEdit
}

internal fun canonicalToggle(
    config: CanonicalConfig,
    field: CanonicalToggle,
): Boolean =
    when (field) {
        CanonicalToggle.Debug -> config.debug
        CanonicalToggle.DebugSwitch -> config.debugSwitch
        CanonicalToggle.RememberSuperkey -> config.settings.rememberSuperkey
        CanonicalToggle.AutoHideServices -> config.settings.autoHideVpnServices
        CanonicalToggle.AutoHideName -> config.settings.autoHideVpnName
    }

internal fun canonicalSet(
    config: CanonicalConfig,
    field: CanonicalSetField,
): Set<String> =
    when (field) {
        CanonicalSetField.OptionalFeatures -> config.settings.optionalFeatures
        CanonicalSetField.AutoHideExcludedPackages -> config.settings.autoHideExcludedPackages
        CanonicalSetField.AutoHiddenPackages -> config.settings.autoHiddenPackages
    }

/** Turn an editor's before/after pair into field edits; never submit its full old snapshot as a write. */
internal fun canonicalEdits(
    base: CanonicalConfig,
    desired: CanonicalConfig,
): List<CanonicalEdit> =
    buildList {
        val next = canonicalConfigSnapshot(desired)
        CanonicalToggle.entries.forEach {
            if (canonicalToggle(base, it) !=
                canonicalToggle(next, it)
            ) {
                add(CanonicalEdit.Toggle(it, canonicalToggle(next, it)))
            }
        }
        CanonicalSetField.entries.forEach {
            if (canonicalSet(base, it) !=
                canonicalSet(next, it)
            ) {
                add(CanonicalEdit.SettingSet(it, canonicalSet(next, it)))
            }
        }
        (base.apps.keys + next.apps.keys).forEach { pkg ->
            val previous = base.apps[pkg] ?: CanonicalApp()
            val app = next.apps[pkg] ?: CanonicalApp()
            CanonicalAppField.entries.forEach { field ->
                if (mergeCanonicalApp(previous, app, field) != previous) add(CanonicalEdit.App(pkg, field, app))
            }
        }
    }

internal fun canonicalEditFields(edit: CanonicalEdit): Set<ConfigField> =
    when (edit) {
        is CanonicalEdit.Toggle -> setOf(ConfigField(edit.field.path))
        is CanonicalEdit.SettingSet -> setOf(ConfigField(listOf("settings", edit.field.name.replaceFirstChar(Char::lowercase))))
        is CanonicalEdit.App -> setOf(ConfigField(listOf("apps", edit.pkg, edit.field.name.replaceFirstChar(Char::lowercase))))
        is CanonicalEdit.RemoveApp -> setOf(ConfigField(listOf("apps", edit.pkg)))
        is CanonicalEdit.Replace -> setOf("apps", "settings", "debug", "debugSwitch").mapTo(linkedSetOf()) { ConfigField(listOf(it)) }
    }

internal fun applyCanonicalEdit(
    config: CanonicalConfig,
    edit: CanonicalEdit,
): CanonicalConfig =
    when (edit) {
        is CanonicalEdit.Toggle -> {
            when (edit.field) {
                CanonicalToggle.Debug -> config.copy(debug = edit.enabled)
                CanonicalToggle.DebugSwitch -> config.copy(debugSwitch = edit.enabled)
                CanonicalToggle.RememberSuperkey -> config.copy(settings = config.settings.copy(rememberSuperkey = edit.enabled))
                CanonicalToggle.AutoHideServices -> config.copy(settings = config.settings.copy(autoHideVpnServices = edit.enabled))
                CanonicalToggle.AutoHideName -> config.copy(settings = config.settings.copy(autoHideVpnName = edit.enabled))
            }
        }

        is CanonicalEdit.SettingSet -> {
            config.copy(
                settings =
                    when (edit.field) {
                        CanonicalSetField.OptionalFeatures -> config.settings.copy(optionalFeatures = edit.values.toSet())
                        CanonicalSetField.AutoHideExcludedPackages -> config.settings.copy(autoHideExcludedPackages = edit.values.toSet())
                        CanonicalSetField.AutoHiddenPackages -> config.settings.copy(autoHiddenPackages = edit.values.toSet())
                    },
            )
        }

        is CanonicalEdit.App -> {
            val app = mergeCanonicalApp(config.apps[edit.pkg] ?: CanonicalApp(), edit.value, edit.field)
            config.copy(apps = if (app.hasAnyRole) config.apps + (edit.pkg to app) else config.apps - edit.pkg)
        }

        is CanonicalEdit.RemoveApp -> {
            config.copy(apps = config.apps - edit.pkg)
        }

        is CanonicalEdit.Replace -> {
            canonicalConfigSnapshot(edit.config)
        }
    }

private fun mergeCanonicalApp(
    base: CanonicalApp,
    next: CanonicalApp,
    field: CanonicalAppField,
): CanonicalApp =
    when (field) {
        CanonicalAppField.Java -> base.copy(java = next.java, javaHooks = next.javaHooks?.toList())
        CanonicalAppField.Native -> base.copy(native = next.native)
        CanonicalAppField.AppHiding -> base.copy(appHiding = next.appHiding)
        CanonicalAppField.Ports -> base.copy(ports = next.ports, portPolicy = next.portPolicy)
        CanonicalAppField.Hidden -> base.copy(hidden = next.hidden)
    }

/** Effect input, deliberately not a data class: secret commands must not appear in toString/state. */
internal class CanonicalMutation(
    edits: List<CanonicalEdit>,
    val source: OperationSource = OperationSource.Ui,
    val activation: CanonicalActivation = CanonicalActivation(),
    coupledCommands: List<String> = emptyList(),
    val bootstrap: Boolean = false,
    val forceActivation: Boolean = false,
    val transform: (CanonicalConfig) -> CanonicalConfig = { it },
) {
    val edits: List<CanonicalEdit> = edits.map(::canonicalEditSnapshot)
    val coupledCommands: List<String> = coupledCommands.toList()
    val writes: Set<ConfigField> = this.edits.flatMapTo(linkedSetOf(), ::canonicalEditFields)
    val protectsDrafts: Boolean = source == OperationSource.System || this.edits.any { it is CanonicalEdit.Replace }
}

internal fun applyCanonicalMutation(
    base: CanonicalConfig,
    mutation: CanonicalMutation,
): CanonicalConfig = canonicalConfigSnapshot(mutation.transform(mutation.edits.fold(canonicalConfigSnapshot(base), ::applyCanonicalEdit)))

internal fun canonicalEditSnapshot(edit: CanonicalEdit): CanonicalEdit =
    when (edit) {
        is CanonicalEdit.SettingSet -> {
            edit.copy(values = edit.values.toSet())
        }

        is CanonicalEdit.App -> {
            edit.copy(
                value = canonicalConfigSnapshot(CanonicalConfig(apps = mapOf(edit.pkg to edit.value))).apps.getValue(edit.pkg),
            )
        }

        is CanonicalEdit.Replace -> {
            edit.copy(config = canonicalConfigSnapshot(edit.config))
        }

        else -> {
            edit
        }
    }
