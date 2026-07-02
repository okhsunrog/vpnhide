package dev.okhsunrog.vpnhide

import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.Process
import dev.okhsunrog.vpnhide.generated.HookIds
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

internal const val LSPOSED_STATE_FILE = "/data/system/vpnhide_lsposed_state"
internal const val LEGACY_HOOK_STATUS_FILE = "/data/system/vpnhide_hook_active"

internal object LsposedStateMetadata {
    const val VERSION = "version"
    const val BOOT_ID = "boot_id"
    const val TIMESTAMP = "timestamp"
    const val AOSP_SDK = "aosp_sdk"
    const val BROKEN_FIELDS = "broken_fields"
    const val INSTALL_FAILURES = "install_failures"
}

internal fun parseLsposedStateMetadata(raw: String): Map<String, String> =
    raw
        .lineSequence()
        .map { it.trim() }
        .filter { it.startsWith("meta ") }
        .mapNotNull { line ->
            val parts = line.split(' ', limit = 3)
            val key = parts.getOrNull(1)?.takeIf { it.isNotBlank() } ?: return@mapNotNull null
            val value = parts.getOrNull(2).orEmpty()
            key to value
        }.toMap()

internal fun formatLsposedState(
    status: Protocol.Status,
    metadata: Map<String, String>,
    stats: List<Protocol.StatEntry>,
): String =
    buildString {
        append(Protocol.formatStatus(status))
        for ((key, value) in metadata) {
            append("meta ")
                .append(key)
                .append(' ')
                .append(value)
                .append('\n')
        }
        append(Protocol.formatStats(stats))
    }

internal object LsposedStats {
    // Coalesce disk writes. record() runs on the hot path inside system_server
    // (potentially every NetworkCapabilities/connectivity call from a probing
    // app); a full state-file rewrite per call is write-amplification + lock
    // contention. Counters stay live in memory and the file is flushed at most
    // once per WRITE_DEBOUNCE_MS on a dedicated thread. setStatus() (rare) still
    // flushes promptly so install status is published without delay.
    private const val WRITE_DEBOUNCE_MS = 1_000L

    private val counts = ConcurrentHashMap<Int, ConcurrentHashMap<Int, AtomicLong>>()

    private val writeHandler: Handler by lazy {
        Handler(HandlerThread("vpnhide-stats-writer").apply { start() }.looper)
    }
    private val flushPending = AtomicBoolean(false)
    private val flushRunnable =
        Runnable {
            flushPending.set(false)
            writeState()
        }

    @Volatile private var installedHooks: Int = 0

    @Volatile private var brokenFields: List<String> = emptyList()

    @Volatile private var installFailures: List<String> = emptyList()

    fun setStatus(
        installedHookMask: Int,
        broken: List<String>,
        failures: List<String> = emptyList(),
    ) {
        installedHooks = installedHookMask
        brokenFields = broken
        installFailures = failures.map(::sanitizeMetadataValue).take(MAX_METADATA_FAILURES)
        // Supersede any coalesced counter write with an immediate flush.
        writeHandler.removeCallbacks(flushRunnable)
        flushPending.set(false)
        writeHandler.post(flushRunnable)
    }

    fun record(
        uid: Int,
        hook: HookIds.Hook,
    ) {
        if (uid < Process.FIRST_APPLICATION_UID) return
        counts
            .computeIfAbsent(uid) { ConcurrentHashMap() }
            .computeIfAbsent(hook.id) { AtomicLong() }
            .incrementAndGet()
        scheduleWrite()
    }

    private fun scheduleWrite() {
        if (flushPending.compareAndSet(false, true)) {
            writeHandler.postDelayed(flushRunnable, WRITE_DEBOUNCE_MS)
        }
    }

    @Synchronized
    private fun writeState() {
        try {
            val text = buildStateText()
            val tmp = File("$LSPOSED_STATE_FILE.tmp.${Process.myPid()}")
            tmp.writeText(text)
            if (!tmp.renameTo(File(LSPOSED_STATE_FILE))) {
                tmp.delete()
                File(LSPOSED_STATE_FILE).writeText(text)
            }
        } catch (t: Throwable) {
            HookLog.e("VpnHide: failed to write LSPosed state: ${t.message}")
        }
    }

    private fun buildStateText(): String {
        val installed = installedHooks
        val error =
            if (installed == HookIds.LSPOSED_HOOK_MASK) {
                HookIds.StatusError.OK.code
            } else {
                HookIds.StatusError.PARTIAL_HOOKS.code
            }
        val status =
            Protocol.Status(
                backend =
                    HookIds.Backend.LSPOSED.id
                        .toLong(),
                kver = 0,
                hooks = installed.toLong(),
                error = error.toLong(),
            )
        return formatLsposedState(
            status = status,
            metadata = metadataSnapshot(),
            stats = statsSnapshot(),
        )
    }

    private fun metadataSnapshot(): Map<String, String> {
        val meta =
            linkedMapOf(
                LsposedStateMetadata.VERSION to BuildConfig.VERSION_NAME,
                LsposedStateMetadata.BOOT_ID to readBootId(),
                LsposedStateMetadata.TIMESTAMP to (System.currentTimeMillis() / 1000).toString(),
                LsposedStateMetadata.AOSP_SDK to Build.VERSION.SDK_INT.toString(),
            )
        val broken = brokenFields
        if (broken.isNotEmpty()) {
            meta[LsposedStateMetadata.BROKEN_FIELDS] = broken.joinToString(",")
        }
        val failures = installFailures
        if (failures.isNotEmpty()) {
            meta[LsposedStateMetadata.INSTALL_FAILURES] = failures.joinToString("; ")
        }
        return meta
    }

    private fun statsSnapshot(): List<Protocol.StatEntry> =
        counts
            .toSortedMap()
            .flatMap { (uid, byHook) ->
                byHook
                    .toSortedMap()
                    .mapNotNull { (hook, count) ->
                        val value = count.get()
                        if (value == 0L) {
                            null
                        } else {
                            Protocol.StatEntry(uid.toLong(), hook.toLong(), value)
                        }
                    }
            }

    private fun readBootId(): String =
        try {
            File("/proc/sys/kernel/random/boot_id").readText().trim()
        } catch (_: Throwable) {
            ""
        }

    private fun sanitizeMetadataValue(value: String): String =
        value
            .lineSequence()
            .joinToString(" ") { it.trim() }
            .replace(Regex("\\s+"), " ")
            .take(MAX_METADATA_VALUE_CHARS)

    private const val MAX_METADATA_FAILURES = 24
    private const val MAX_METADATA_VALUE_CHARS = 240
}
