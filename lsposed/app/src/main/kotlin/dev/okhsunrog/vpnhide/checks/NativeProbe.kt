package dev.okhsunrog.vpnhide.checks

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/**
 * Native detection-probe outcome, shared by both transports.
 *
 * Replaces the UniFFI-generated types: the whole native surface is now one
 * JSON-returning function ([NativeProbe.runAllChecksJson]), so the Rust crate
 * builds with plain cargo-ndk (no gobley plugin / AGP-9 fork). The same JSON is
 * produced in-process (app view) and by the root-exec'd `vhprobe` bin (ground
 * truth), so this parser serves both.
 */
enum class CheckStatus { PASS, FAIL, NETWORK_BLOCKED }

data class CheckOutput(
    val status: CheckStatus,
    val detail: String,
)

/** JNI entry to the in-process (app-view) probe run. */
object NativeProbe {
    init {
        System.loadLibrary("vpnhide_checks")
    }

    /** Runs every native probe in this process and returns a JSON array of
     * `{id, status, detail}`. Backed by the Rust `run_all_json`. */
    external fun runAllChecksJson(): String

    /** In-process (app-view) run: probes execute as this app (real uid +
     * SELinux domain + zygisk/kernel hooks), keyed by stable check id. */
    fun runAll(): Map<String, CheckOutput> = parse(runAllChecksJson())

    /** Parse a probe JSON blob (from either transport) into id -> outcome. */
    fun parse(json: String): Map<String, CheckOutput> =
        runCatching {
            probeJson.decodeFromString<List<CheckJson>>(json).associate { c ->
                c.id to CheckOutput(statusOf(c.status), c.detail)
            }
        }.getOrDefault(emptyMap())

    private val probeJson = Json { ignoreUnknownKeys = true }

    private fun statusOf(raw: String): CheckStatus =
        when (raw) {
            "pass" -> CheckStatus.PASS
            "fail" -> CheckStatus.FAIL
            else -> CheckStatus.NETWORK_BLOCKED
        }

    @Serializable
    private data class CheckJson(
        val id: String,
        val status: String,
        val detail: String,
    )
}
