package dev.okhsunrog.vpnhide

import uniffi.vpnhide_core_ffi.CoreException
import uniffi.vpnhide_core_ffi.CorePortPolicy
import uniffi.vpnhide_core_ffi.CorePortPolicyMode
import uniffi.vpnhide_core_ffi.CorePortProtocol
import uniffi.vpnhide_core_ffi.CorePortRule
import uniffi.vpnhide_core_ffi.CorePortRuleIntent
import uniffi.vpnhide_core_ffi.normalizedPortPolicy
import uniffi.vpnhide_core_ffi.portPolicyPresets
import uniffi.vpnhide_core_ffi.normalizedPortRules as sharedNormalizedPortRules
import uniffi.vpnhide_core_ffi.parseAgentPortPolicy as sharedAgentPortPolicy
import uniffi.vpnhide_core_ffi.portPolicyForPreset as sharedPresetPolicy

/** App-only local FFI adapters; the tolerant system_server reader never calls these. */
internal fun loadPortPolicyPresets(): List<PortPolicyPreset> =
    portPolicyPresets().map { PortPolicyPreset(it.id, it.rules.map(CorePortRule::toCanonical)) }

internal fun portPolicyForPreset(id: String): PortPolicy? = sharedPresetPolicy(id)?.toCanonical()

internal fun normalizePortPolicy(policy: PortPolicy?): PortPolicy? = normalizedPortPolicy(policy?.toCore())?.toCanonical()

internal fun normalizedPortRules(rules: List<PortRule>): List<PortRule> =
    sharedNormalizedPortRules(rules.map(PortRule::toCore)).map(CorePortRule::toCanonical)

internal fun parseAgentPortPolicyResult(
    mode: String,
    preset: String?,
    rules: List<AgentPortRule>,
): Result<PortPolicy?> =
    runCatching {
        try {
            sharedAgentPortPolicy(
                mode,
                preset,
                rules.map { CorePortRuleIntent(it.protocol, it.start.toLong(), it.end.toLong()) },
            )?.toCanonical()
        } catch (error: CoreException.InvalidInput) {
            throw IllegalArgumentException(error.detail)
        }
    }

internal fun PortPolicy.toCore(): CorePortPolicy =
    CorePortPolicy(
        when (mode) {
            PortPolicyMode.Preset -> CorePortPolicyMode.Preset
            PortPolicyMode.Custom -> CorePortPolicyMode.Custom
        },
        preset,
        rules.map(PortRule::toCore),
    )

internal fun CorePortPolicy.toCanonical(): PortPolicy =
    PortPolicy(
        when (mode) {
            CorePortPolicyMode.Preset -> PortPolicyMode.Preset
            CorePortPolicyMode.Custom -> PortPolicyMode.Custom
            is CorePortPolicyMode.Unknown -> error("Unsupported port policy mode")
        },
        preset,
        rules.map(CorePortRule::toCanonical),
    )

internal fun PortRule.toCore(): CorePortRule = CorePortRule(protocol.toCore(), start.toUShort(), end.toUShort())

internal fun CorePortRule.toCanonical(): PortRule = PortRule(protocol.toCanonical(), start.toInt(), end.toInt())

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
