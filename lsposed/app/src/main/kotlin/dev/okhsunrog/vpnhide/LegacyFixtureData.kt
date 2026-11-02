@file:Suppress("DEPRECATION")

package dev.okhsunrog.vpnhide

import android.net.ConnectivityManager
import android.net.NetworkInfo
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import java.security.MessageDigest

/** Getter projection avoids invoking the parcel sanitizer while recording its baseline. */
@Serializable
internal data class LegacyInfoFields(
    val type: Int,
    val subtype: Int,
    val typeName: String?,
    val subtypeName: String?,
    val state: String,
    val detailedState: String,
    val available: Boolean,
    val failover: Boolean,
    val roaming: Boolean,
    val reason: String?,
    val extraInfo: String?,
)

internal fun legacyInfoFields(info: NetworkInfo): LegacyInfoFields =
    LegacyInfoFields(
        info.type,
        info.subtype,
        info.typeName,
        info.subtypeName,
        info.state.name,
        info.detailedState.name,
        info.isAvailable,
        info.isFailover,
        info.isRoaming,
        info.reason,
        info.extraInfo,
    )

internal data class LegacyOrdinaryInfo(
    val count: Int,
    val sha256: String,
    val fixturePresent: Boolean,
)

/** Multiset hashing preserves duplicates and detects changes to ordinary NetworkInfo. */
internal fun legacyOrdinaryInfo(
    infos: List<LegacyInfoFields>,
    ordinaryMarker: String,
): LegacyOrdinaryInfo {
    require(infos.size <= 256)
    val ordinary = infos.filter { it.type != ConnectivityManager.TYPE_VPN }
    val encoded =
        ordinary
            .map { fields ->
                Json.encodeToString(fields).also { require(it.length <= 8192) }
            }.sorted()
    val bytes = Json.encodeToString(encoded).toByteArray()
    return LegacyOrdinaryInfo(
        ordinary.size,
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) },
        ordinary.any { it.extraInfo == ordinaryMarker },
    )
}

internal data class LegacySourceArray<T>(
    val infos: List<T>,
    val ordinary: LegacyOrdinaryInfo,
)

/** Preserve every original entry and refuse ambiguous/replayed fixture markers. */
internal fun <T : Any> appendLegacyFixture(
    original: List<T?>,
    vpn: T,
    ordinary: T,
    vpnMarker: String,
    ordinaryMarker: String,
    fields: (T) -> LegacyInfoFields,
): LegacySourceArray<T>? {
    if (original.size > 254 || original.any { it == null }) return null
    val originals = original.filterNotNull()
    if (originals.any { fields(it).extraInfo in setOf(vpnMarker, ordinaryMarker) }) return null
    val next = originals + listOf(vpn, ordinary)
    return LegacySourceArray(next, legacyOrdinaryInfo(next.map(fields), ordinaryMarker))
}
