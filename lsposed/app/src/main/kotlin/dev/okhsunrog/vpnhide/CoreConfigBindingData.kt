package dev.okhsunrog.vpnhide

import uniffi.vpnhide_core_ffi.CoreAppConfig
import uniffi.vpnhide_core_ffi.CoreCanonicalConfig
import uniffi.vpnhide_core_ffi.CoreConfigAppField
import uniffi.vpnhide_core_ffi.CoreConfigSetField
import uniffi.vpnhide_core_ffi.CoreConfigToggle
import uniffi.vpnhide_core_ffi.CoreEdit
import uniffi.vpnhide_core_ffi.CoreFieldEdit
import uniffi.vpnhide_core_ffi.CoreNativeRole
import uniffi.vpnhide_core_ffi.CorePortPolicy
import uniffi.vpnhide_core_ffi.CorePortPolicyMode
import uniffi.vpnhide_core_ffi.CorePortProtocol
import uniffi.vpnhide_core_ffi.CorePortRule
import uniffi.vpnhide_core_ffi.CoreSettings

/** The hook's file model crosses the app boundary as generated records, with no JSON control codec. */
internal fun CanonicalConfig.toCore(): CoreCanonicalConfig =
    CoreCanonicalConfig(
        version.toUInt(),
        debug,
        debugSwitch,
        apps.mapValues { it.value.toCore() },
        CoreSettings(
            settings.rememberSuperkey,
            settings.optionalFeatures.sorted(),
            settings.autoHideVpnServices,
            settings.autoHideVpnName,
            settings.autoHideExcludedPackages.sorted(),
            settings.autoHiddenPackages.sorted(),
        ),
    )

internal fun CoreCanonicalConfig.toCanonical(): CanonicalConfig =
    CanonicalConfig(
        version.toInt(),
        debug,
        debugSwitch,
        apps.mapValues { it.value.toCanonical() },
        CanonicalSettings(
            settings.rememberSuperkey,
            settings.optionalFeatures.toSet(),
            settings.autoHideVpnServices,
            settings.autoHideVpnName,
            settings.autoHideExcludedPackages.toSet(),
            settings.autoHiddenPackages.toSet(),
        ),
    )

private fun CanonicalApp.toCore(): CoreAppConfig =
    CoreAppConfig(
        java,
        javaHooks,
        CoreNativeRole(native.enabled, native.overrides.kernel, native.overrides.zygisk),
        appHiding,
        ports,
        portPolicy?.let { policy ->
            CorePortPolicy(
                when (policy.mode) {
                    PortPolicyMode.Preset -> CorePortPolicyMode.Preset
                    PortPolicyMode.Custom -> CorePortPolicyMode.Custom
                },
                policy.preset,
                policy.rules.map { CorePortRule(it.protocol.toCore(), it.start.toUShort(), it.end.toUShort()) },
            )
        },
        hidden,
    )

private fun CoreAppConfig.toCanonical(): CanonicalApp =
    CanonicalApp(
        java,
        javaHooks,
        NativeRole(native.enabled, NativeHookOverrides(native.kernel, native.zygisk)),
        appHiding,
        ports,
        portPolicy?.let { policy ->
            PortPolicy(
                when (policy.mode) {
                    CorePortPolicyMode.Preset -> PortPolicyMode.Preset
                    CorePortPolicyMode.Custom -> PortPolicyMode.Custom
                    is CorePortPolicyMode.Unknown -> error("Unsupported port policy mode")
                },
                policy.preset,
                policy.rules.map { PortRule(it.protocol.toCanonical(), it.start.toInt(), it.end.toInt()) },
            )
        },
        hidden,
    )

private fun PortProtocol.toCore(): CorePortProtocol =
    when (this) {
        PortProtocol.Both -> CorePortProtocol.BOTH
        PortProtocol.Tcp -> CorePortProtocol.TCP
        PortProtocol.Udp -> CorePortProtocol.UDP
    }

private fun CorePortProtocol.toCanonical(): PortProtocol =
    when (this) {
        CorePortProtocol.BOTH -> PortProtocol.Both
        CorePortProtocol.TCP -> PortProtocol.Tcp
        CorePortProtocol.UDP -> PortProtocol.Udp
    }

internal fun coreEdit(edits: List<CanonicalEdit>): CoreEdit {
    val replacement = edits.filterIsInstance<CanonicalEdit.Replace>().singleOrNull()
    if (replacement != null) {
        require(edits.size == 1)
        return CoreEdit.Replace(replacement.config.toCore())
    }
    return CoreEdit.Batch(edits.map(::coreFieldEdit))
}

private fun coreFieldEdit(edit: CanonicalEdit): CoreFieldEdit =
    when (edit) {
        is CanonicalEdit.Toggle -> CoreFieldEdit.Toggle(edit.field.toCore(), edit.enabled)
        is CanonicalEdit.SettingSet -> CoreFieldEdit.SettingSet(edit.field.toCore(), edit.values.sorted())
        is CanonicalEdit.App -> CoreFieldEdit.App(edit.pkg, edit.field.toCore(), edit.value.toCore())
        is CanonicalEdit.RemoveApp -> CoreFieldEdit.RemoveApp(edit.pkg)
        is CanonicalEdit.Replace -> error("Replace cannot be included in a field batch")
    }

private fun CanonicalToggle.toCore(): CoreConfigToggle =
    when (this) {
        CanonicalToggle.Filesystem -> CoreConfigToggle.FILESYSTEM
        CanonicalToggle.Debug -> CoreConfigToggle.DEBUG
        CanonicalToggle.DebugSwitch -> CoreConfigToggle.DEBUG_SWITCH
        CanonicalToggle.RememberSuperkey -> CoreConfigToggle.REMEMBER_SUPERKEY
        CanonicalToggle.AutoHideServices -> CoreConfigToggle.AUTO_HIDE_SERVICES
        CanonicalToggle.AutoHideName -> CoreConfigToggle.AUTO_HIDE_NAME
    }

private fun CanonicalSetField.toCore(): CoreConfigSetField =
    when (this) {
        CanonicalSetField.OptionalFeatures -> CoreConfigSetField.OPTIONAL_FEATURES
        CanonicalSetField.AutoHideExcludedPackages -> CoreConfigSetField.AUTO_HIDE_EXCLUDED_PACKAGES
        CanonicalSetField.AutoHiddenPackages -> CoreConfigSetField.AUTO_HIDDEN_PACKAGES
    }

private fun CanonicalAppField.toCore(): CoreConfigAppField =
    when (this) {
        CanonicalAppField.Java -> CoreConfigAppField.JAVA
        CanonicalAppField.Native -> CoreConfigAppField.NATIVE
        CanonicalAppField.AppHiding -> CoreConfigAppField.APP_HIDING
        CanonicalAppField.Ports -> CoreConfigAppField.PORTS
        CanonicalAppField.Hidden -> CoreConfigAppField.HIDDEN
    }
