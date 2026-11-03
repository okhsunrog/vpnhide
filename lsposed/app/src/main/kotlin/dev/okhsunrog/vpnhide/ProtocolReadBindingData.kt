package dev.okhsunrog.vpnhide

import uniffi.vpnhide_core_ffi.CoreProtocolKind
import uniffi.vpnhide_core_ffi.protocolDisplayStats
import uniffi.vpnhide_core_ffi.protocolDisplayStatus
import uniffi.vpnhide_core_ffi.protocolExtractBlock
import uniffi.vpnhide_core_ffi.protocolPeekKind
import uniffi.vpnhide_core_ffi.protocolStatusFields

/** App-only typed adapters; the injected producer never calls these readers. */
internal object ProtocolReader {
    fun peekKind(text: String): Protocol.Kind? = protocolPeekKind(text)?.toLocal()

    fun parseStats(text: String): List<Protocol.StatEntry>? =
        protocolDisplayStats(text)?.map { Protocol.StatEntry(it.uid.toLong(), it.hookId.toLong(), it.count.toLong()) }

    fun parseStatusFields(text: String): Protocol.StatusFields? =
        protocolStatusFields(text)?.let {
            Protocol.StatusFields(it.backend?.toLong(), it.kver?.toLong(), it.hooks?.toLong(), it.error?.toLong())
        }

    fun parseStatus(text: String): Protocol.Status? =
        protocolDisplayStatus(text)?.let { Protocol.Status(it.backend.toLong(), it.kver.toLong(), it.hooks.toLong(), it.error.toLong()) }
}

internal fun extractTelemetryBlock(
    raw: String,
    kind: Protocol.Kind,
): String? = protocolExtractBlock(raw, kind.toCore())

private fun CoreProtocolKind.toLocal(): Protocol.Kind =
    when (this) {
        CoreProtocolKind.CONFIG -> Protocol.Kind.CONFIG
        CoreProtocolKind.STATS -> Protocol.Kind.STATS
        CoreProtocolKind.STATUS -> Protocol.Kind.STATUS
    }

private fun Protocol.Kind.toCore(): CoreProtocolKind =
    when (this) {
        Protocol.Kind.CONFIG -> CoreProtocolKind.CONFIG
        Protocol.Kind.STATS -> CoreProtocolKind.STATS
        Protocol.Kind.STATUS -> CoreProtocolKind.STATUS
    }
