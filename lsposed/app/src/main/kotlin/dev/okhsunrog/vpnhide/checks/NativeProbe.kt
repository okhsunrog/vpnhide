package dev.okhsunrog.vpnhide.checks

import android.util.Log
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull

private const val OBSERVATION_VERSION = 1

/** Native result status. Unknown future statuses stay visible as UNKNOWN. */
enum class CheckStatus { PASS, FAIL, SELINUX_BLOCKED, NETWORK_BLOCKED, UNKNOWN }

data class CheckOutput(
    val status: CheckStatus,
    val detail: String,
)

/** Errors in the app/helper observation envelope. */
sealed interface ObservationError {
    data object Malformed : ObservationError

    data class UnsupportedVersion(
        val version: Long,
    ) : ObservationError

    data class WrongKind(
        val kind: String,
    ) : ObservationError

    data class UnknownStatus(
        val status: String,
    ) : ObservationError

    data object Unavailable : ObservationError

    data class UnknownError(
        val error: String,
    ) : ObservationError
}

sealed interface ChecksResponse {
    data class Success(
        val checks: Map<String, CheckOutput>,
    ) : ChecksResponse

    data class Failure(
        val error: ObservationError,
    ) : ChecksResponse
}

data class RoutingObservation(
    val uid: Long,
    val routed: Boolean?,
    val detail: String,
)

enum class AppVpnState { VPN_OFF, EXCLUDED, ROUTED, UNKNOWN }

data class AppVpnStateObservation(
    val uid: Long,
    val state: AppVpnState,
    val session: String?,
    val interfaces: List<String>,
    val method: String,
    val detail: String,
)

sealed interface AppVpnStateResponse {
    data class Success(
        val observation: AppVpnStateObservation,
    ) : AppVpnStateResponse

    data class Failure(
        val error: ObservationError,
    ) : AppVpnStateResponse
}

sealed interface RoutingResponse {
    data class Success(
        val observation: RoutingObservation,
    ) : RoutingResponse

    data class Failure(
        val error: ObservationError,
    ) : RoutingResponse
}

data class KpmListObservation(
    val available: Boolean,
    val modules: Set<String>,
)

sealed interface KpmListResponse {
    data class Success(
        val observation: KpmListObservation,
    ) : KpmListResponse

    data class Failure(
        val error: ObservationError,
    ) : KpmListResponse
}

/** JNI entry to the in-process (app-view) probe run. */
object NativeProbe {
    private const val CHECKS_KIND = "checks"
    private const val ROUTING_KIND = "routing"
    private const val APP_VPN_STATE_KIND = "app_vpn_state"
    private const val KPM_LIST_KIND = "kpm_list"
    private val probeJson = Json { ignoreUnknownKeys = true }

    // Keep parsing usable in JVM unit tests that do not have an Android JNI
    // library. The library is loaded only for the actual in-process run.
    private val nativeLibrary = lazy { System.loadLibrary("vpnhide_checks") }

    /** Runs every native probe in this process and returns the versioned envelope. */
    external fun runAllChecksJson(): String

    /** In-process (app-view) run: probes execute as this app (real uid +
     * SELinux domain + zygisk/kernel hooks), keyed by stable check id. */
    fun runAll(): Map<String, CheckOutput> =
        runCatching {
            nativeLibrary.value
            runAllChecksJson()
        }.onFailure { Log.e("VpnHide-Native", "native probe run failed", it) }
            .getOrElse { return emptyMap() }
            .let { json ->
                when (val response = parseChecks(json)) {
                    is ChecksResponse.Success -> {
                        response.checks
                    }

                    is ChecksResponse.Failure -> {
                        Log.e("VpnHide-Native", "native observation rejected: ${response.error}")
                        emptyMap()
                    }
                }
            }

    /** Parse a checks response from either JNI or root helper transport. */
    fun parseChecks(json: String): ChecksResponse =
        when (val envelope = parseEnvelope(json, CHECKS_KIND)) {
            is Envelope.Success -> parseChecksData(envelope.data)
            is Envelope.Failure -> ChecksResponse.Failure(envelope.error)
        }

    /** Parse the legacy UID-routing response from the root helper transport. */
    fun parseRouting(json: String): RoutingResponse =
        when (val envelope = parseEnvelope(json, ROUTING_KIND)) {
            is Envelope.Success -> parseRoutingData(envelope.data)
            is Envelope.Failure -> RoutingResponse.Failure(envelope.error)
        }

    /** Parse the combined VPN-presence and self-UID routing observation. */
    fun parseAppVpnState(json: String): AppVpnStateResponse =
        when (val envelope = parseEnvelope(json, APP_VPN_STATE_KIND)) {
            is Envelope.Success -> parseAppVpnStateData(envelope.data)
            is Envelope.Failure -> AppVpnStateResponse.Failure(envelope.error)
        }

    /** Parse the structured runtime KPM listing used by the root snapshot. */
    fun parseKpmList(json: String): KpmListResponse =
        when (val envelope = parseEnvelope(json, KPM_LIST_KIND)) {
            is Envelope.Success -> parseKpmListData(envelope.data)
            is Envelope.Failure -> KpmListResponse.Failure(envelope.error)
        }

    /** Compatibility projection for callers that only need check values. */
    fun parse(json: String): Map<String, CheckOutput> = (parseChecks(json) as? ChecksResponse.Success)?.checks.orEmpty()

    private fun parseChecksData(data: JsonElement): ChecksResponse {
        val array = data as? JsonArray ?: return ChecksResponse.Failure(ObservationError.Malformed)
        val checks = linkedMapOf<String, CheckOutput>()
        for (element in array) {
            val item = element as? JsonObject ?: return ChecksResponse.Failure(ObservationError.Malformed)
            val id = item.stringField("id") ?: return ChecksResponse.Failure(ObservationError.Malformed)
            if (id.isEmpty()) return ChecksResponse.Failure(ObservationError.Malformed)
            val status = item.stringField("status") ?: return ChecksResponse.Failure(ObservationError.Malformed)
            val detail = item.stringField("detail") ?: return ChecksResponse.Failure(ObservationError.Malformed)
            if (checks.put(id, CheckOutput(statusOf(status), detail)) != null) {
                return ChecksResponse.Failure(ObservationError.Malformed)
            }
        }
        return ChecksResponse.Success(checks)
    }

    private fun parseRoutingData(data: JsonElement): RoutingResponse {
        val item = data as? JsonObject ?: return RoutingResponse.Failure(ObservationError.Malformed)
        val uid = item.longField("uid") ?: return RoutingResponse.Failure(ObservationError.Malformed)
        if (uid < 0) return RoutingResponse.Failure(ObservationError.Malformed)
        if (!item.containsKey("routed")) return RoutingResponse.Failure(ObservationError.Malformed)
        val routedElement = item["routed"]
        val routed =
            when (routedElement) {
                JsonNull -> null
                is JsonPrimitive -> routedElement.takeUnless(JsonPrimitive::isString)?.booleanOrNull
                else -> null
            }
        if (routedElement !is JsonNull && routed == null) {
            return RoutingResponse.Failure(ObservationError.Malformed)
        }
        val detail = item.stringField("detail") ?: return RoutingResponse.Failure(ObservationError.Malformed)
        return RoutingResponse.Success(RoutingObservation(uid, routed, detail))
    }

    private fun parseAppVpnStateData(data: JsonElement): AppVpnStateResponse {
        val item = data as? JsonObject ?: return AppVpnStateResponse.Failure(ObservationError.Malformed)
        val uid = item.longField("uid") ?: return AppVpnStateResponse.Failure(ObservationError.Malformed)
        if (uid < 0) return AppVpnStateResponse.Failure(ObservationError.Malformed)
        val state =
            when (item.stringField("state")) {
                "vpn_off" -> AppVpnState.VPN_OFF
                "excluded" -> AppVpnState.EXCLUDED
                "routed" -> AppVpnState.ROUTED
                "unknown" -> AppVpnState.UNKNOWN
                else -> return AppVpnStateResponse.Failure(ObservationError.Malformed)
            }
        val sessionElement = item["session"]
        val session =
            when (sessionElement) {
                JsonNull -> null
                is JsonPrimitive -> sessionElement.takeIf(JsonPrimitive::isString)?.content
                else -> null
            }
        if (sessionElement !is JsonNull && session == null) {
            return AppVpnStateResponse.Failure(ObservationError.Malformed)
        }
        val interfaces =
            (item["interfaces"] as? JsonArray)
                ?.map { element ->
                    (element as? JsonPrimitive)?.takeIf(JsonPrimitive::isString)?.content
                        ?: return AppVpnStateResponse.Failure(ObservationError.Malformed)
                } ?: return AppVpnStateResponse.Failure(ObservationError.Malformed)
        if (interfaces.any(String::isEmpty)) return AppVpnStateResponse.Failure(ObservationError.Malformed)
        val method = item.stringField("method") ?: return AppVpnStateResponse.Failure(ObservationError.Malformed)
        val detail = item.stringField("detail") ?: return AppVpnStateResponse.Failure(ObservationError.Malformed)
        return AppVpnStateResponse.Success(AppVpnStateObservation(uid, state, session, interfaces, method, detail))
    }

    private fun parseKpmListData(data: JsonElement): KpmListResponse {
        val item = data as? JsonObject ?: return KpmListResponse.Failure(ObservationError.Malformed)
        val available =
            (item["available"] as? JsonPrimitive)?.let { primitive ->
                if (primitive.isString) null else primitive.booleanOrNull
            } ?: return KpmListResponse.Failure(ObservationError.Malformed)
        val modules = item["modules"] as? JsonArray ?: return KpmListResponse.Failure(ObservationError.Malformed)
        if (!available) return KpmListResponse.Failure(ObservationError.Malformed)
        val names = linkedSetOf<String>()
        for (module in modules) {
            val name =
                (module as? JsonPrimitive)?.takeIf { it.isString }?.content
                    ?: return KpmListResponse.Failure(ObservationError.Malformed)
            if (name.isEmpty()) return KpmListResponse.Failure(ObservationError.Malformed)
            names += name
        }
        return KpmListResponse.Success(KpmListObservation(available = true, modules = names))
    }

    private fun statusOf(raw: String): CheckStatus =
        when (raw) {
            "pass" -> CheckStatus.PASS
            "fail" -> CheckStatus.FAIL
            "selinux_blocked" -> CheckStatus.SELINUX_BLOCKED
            "network_blocked" -> CheckStatus.NETWORK_BLOCKED
            else -> CheckStatus.UNKNOWN
        }

    private sealed interface Envelope {
        data class Success(
            val data: JsonElement,
        ) : Envelope

        data class Failure(
            val error: ObservationError,
        ) : Envelope
    }

    private fun parseEnvelope(
        json: String,
        expectedKind: String,
    ): Envelope {
        val objectValue =
            runCatching { probeJson.parseToJsonElement(json) as? JsonObject }
                .getOrNull()
                ?: return Envelope.Failure(ObservationError.Malformed)
        val version = objectValue.longField("version") ?: return Envelope.Failure(ObservationError.Malformed)
        if (version != OBSERVATION_VERSION.toLong()) {
            return Envelope.Failure(ObservationError.UnsupportedVersion(version))
        }
        val kind = objectValue.stringField("kind") ?: return Envelope.Failure(ObservationError.Malformed)
        if (kind != expectedKind) return Envelope.Failure(ObservationError.WrongKind(kind))
        return when (val status = objectValue.stringField("status")) {
            "ok" -> {
                objectValue["data"]?.let(Envelope::Success)
                    ?: Envelope.Failure(ObservationError.Malformed)
            }

            "error" -> {
                parseError(objectValue)
            }

            null -> {
                Envelope.Failure(ObservationError.Malformed)
            }

            else -> {
                Envelope.Failure(ObservationError.UnknownStatus(status))
            }
        }
    }

    private fun parseError(objectValue: JsonObject): Envelope =
        when (val code = objectValue.stringField("error")) {
            null -> Envelope.Failure(ObservationError.Malformed)
            "unavailable" -> Envelope.Failure(ObservationError.Unavailable)
            "malformed" -> Envelope.Failure(ObservationError.Malformed)
            else -> Envelope.Failure(ObservationError.UnknownError(code))
        }

    private fun JsonObject.stringField(name: String): String? =
        (this[name] as? JsonPrimitive)
            ?.takeIf(JsonPrimitive::isString)
            ?.content

    private fun JsonObject.longField(name: String): Long? =
        (this[name] as? JsonPrimitive)
            ?.takeUnless(JsonPrimitive::isString)
            ?.content
            ?.toLongOrNull()
}
