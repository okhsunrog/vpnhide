package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreRootEffect
import uniffi.vpnhide_core_ffi.canonicalEditPaths
import uniffi.vpnhide_core_ffi.canonicalFieldDiff
import uniffi.vpnhide_core_ffi.canonicalSetValue
import uniffi.vpnhide_core_ffi.canonicalToggleValue
import uniffi.vpnhide_core_ffi.previewCanonicalEdit

internal enum class CanonicalToggle { Filesystem, Debug, DebugSwitch, RememberSuperkey, AutoHideServices, AutoHideName }

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
): Boolean = canonicalToggleValue(config.toCore(), field.toCore())

internal fun canonicalSet(
    config: CanonicalConfig,
    field: CanonicalSetField,
): Set<String> = canonicalSetValue(config.toCore(), field.toCore()).toSet()

/** Shared field diff; only Core may admit effects against the current revision. */
internal fun canonicalEdits(
    base: CanonicalConfig,
    desired: CanonicalConfig,
): List<CanonicalEdit> = canonicalFieldDiff(base.toCore(), desired.toCore()).map { it.toCanonicalEdit() }

internal fun canonicalEditFields(edit: CanonicalEdit): Set<ConfigField> =
    canonicalEditPaths(coreEdit(listOf(edit))).mapTo(linkedSetOf()) { ConfigField(it) }

internal fun applyCanonicalEdit(
    config: CanonicalConfig,
    edit: CanonicalEdit,
): CanonicalConfig = previewCanonicalEdit(config.toCore(), coreEdit(listOf(edit))).toCanonical()

/** Effect input, deliberately not a data class: secret effects must not appear in toString/state. */
internal class CanonicalMutation(
    edits: List<CanonicalEdit>,
    val source: OperationSource = OperationSource.Ui,
    coupledEffects: List<CoreRootEffect> = emptyList(),
    val bootstrap: Boolean = false,
    val declaredWrites: Set<ConfigField> = emptySet(),
    val removesCanonical: Boolean = false,
    val cleanup: Boolean = false,
    val protectAllDrafts: Boolean = false,
    val forceActivation: Boolean = false,
    val transform: (CanonicalConfig) -> CanonicalConfig = { it },
) {
    val edits: List<CanonicalEdit> = edits.map(::canonicalEditSnapshot)
    val coupledEffects: List<CoreRootEffect> = coupledEffects.toList()
    val writes: Set<ConfigField> =
        this.edits.flatMapTo(linkedSetOf(), ::canonicalEditFields) + configFieldSnapshot(declaredWrites) +
            if (protectAllDrafts) setOf("apps", "settings", "debug", "debugSwitch").map { ConfigField(listOf(it)) } else emptySet()
    val protectsDrafts: Boolean = protectAllDrafts || source == OperationSource.System || this.edits.any { it is CanonicalEdit.Replace }
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
