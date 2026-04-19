package dev.okhsunrog.vpnhide

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Per-screen protection state from root-owned files and package
 * manager lookups, cached once for the lifetime of the app session.
 *
 * Without this cache, every tab switch into Protection triggered 3-4
 * sequential `suExec` roundtrips per screen. Root shell roundtrips
 * are ~50-100ms each on most devices, so a single tab switch added
 * hundreds of milliseconds of "loading" time even after AppListCache
 * made the package list itself instant. Bundling every read into a
 * single batched shell invocation + caching the result means subsequent
 * tab switches render immediately from memory.
 *
 * Invalidated when:
 * - The user taps Save on any Protection screen (target files have
 *   just been overwritten — need a fresh read next time).
 * - The user taps the top-bar Refresh button on Protection.
 */
internal data class TargetsSnapshot(
    val kmodModuleInstalled: Boolean,
    val zygiskModuleInstalled: Boolean,
    val portsModuleInstalled: Boolean,
    val kmodTargets: Set<String>,
    val zygiskTargets: Set<String>,
    val lsposedTargets: Set<String>,
    val hiddenPkgs: Set<String>,
    val observerUids: Set<Int>,
    val portsObservers: Set<String>,
    val uidToPkg: Map<Int, String>,
) {
    /** Observer UIDs resolved back to current package names via
     * `pm list packages -U`. UIDs that no longer map to an installed
     * package (e.g. after an uninstall) silently drop out.
     */
    val observerNames: Set<String>
        get() = observerUids.mapNotNull { uidToPkg[it] }.toSet()
}

internal object TargetsCache {
    private val _snapshot = MutableStateFlow<TargetsSnapshot?>(null)
    val snapshot: StateFlow<TargetsSnapshot?> = _snapshot.asStateFlow()

    private val _loading = MutableStateFlow(false)
    val loading: StateFlow<Boolean> = _loading.asStateFlow()

    private var inflight: Job? = null

    fun ensureLoaded(
        scope: CoroutineScope,
        context: Context,
    ) {
        if (_snapshot.value != null || inflight?.isActive == true) return
        inflight = scope.launch { reload(context.applicationContext) }
    }

    fun refresh(
        scope: CoroutineScope,
        context: Context,
    ) {
        inflight?.cancel()
        inflight = scope.launch { reload(context.applicationContext) }
    }

    /** Drop the cached snapshot so the next subscriber triggers a
     * fresh load. Save handlers call this because they just mutated
     * one of the files this cache mirrors.
     */
    fun invalidate() {
        _snapshot.value = null
    }

    // Bundle every read into a single `suExec` call separated by
    // distinctive banner lines. One root roundtrip beats 8 serial
    // ones — PM + 7 cats typically take <100ms this way.
    private const val SENTINEL = "===VPNHIDE-TARGETS-BOUNDARY==="
    private const val END = "===VPNHIDE-TARGETS-END==="

    private val BATCH_SCRIPT =
        """
        echo "$SENTINEL KMOD_MODULE_DIR"
        [ -d $KMOD_MODULE_DIR ] && echo 1 || echo 0
        echo "$SENTINEL ZYGISK_MODULE_DIR"
        [ -d $ZYGISK_MODULE_DIR ] && echo 1 || echo 0
        echo "$SENTINEL PORTS_MODULE_PROP"
        cat $PORTS_MODULE_DIR/module.prop 2>/dev/null || true
        echo "$SENTINEL KMOD_TARGETS"
        cat $KMOD_TARGETS 2>/dev/null || true
        echo "$SENTINEL ZYGISK_TARGETS"
        cat $ZYGISK_TARGETS 2>/dev/null || true
        echo "$SENTINEL LSPOSED_TARGETS"
        cat $LSPOSED_TARGETS 2>/dev/null || true
        echo "$SENTINEL HIDDEN_PKGS"
        cat $SS_HIDDEN_PKGS_FILE 2>/dev/null || true
        echo "$SENTINEL OBSERVER_UIDS"
        cat $SS_OBSERVER_UIDS_FILE 2>/dev/null || true
        echo "$SENTINEL PORTS_OBSERVERS"
        cat $PORTS_OBSERVERS_FILE 2>/dev/null || true
        echo "$SENTINEL PM_LIST"
        pm list packages -U 2>/dev/null || true
        echo "$END"
        """.trimIndent()

    private suspend fun reload(
        @Suppress("UNUSED_PARAMETER") appContext: Context,
    ) {
        _loading.value = true
        try {
            val (_, out) =
                withContext(Dispatchers.IO) { suExec(BATCH_SCRIPT) }
            _snapshot.value = parse(out)
        } finally {
            _loading.value = false
        }
    }

    private fun parse(out: String): TargetsSnapshot {
        val sections = mutableMapOf<String, String>()
        var currentKey: String? = null
        val buf = StringBuilder()
        for (line in out.lines()) {
            when {
                line.startsWith(SENTINEL) -> {
                    currentKey?.let { sections[it] = buf.toString() }
                    buf.clear()
                    currentKey = line.removePrefix("$SENTINEL ").trim()
                }

                line.startsWith(END) -> {
                    currentKey?.let { sections[it] = buf.toString() }
                    currentKey = null
                }

                currentKey != null -> {
                    buf.appendLine(line)
                }
            }
        }

        fun nonEmptyLines(raw: String?): Set<String> =
            raw
                ?.lines()
                ?.map { it.trim() }
                ?.filter { it.isNotEmpty() && !it.startsWith("#") }
                ?.toSet() ?: emptySet()

        val portsInstalled = sections["PORTS_MODULE_PROP"]?.isNotBlank() == true
        val observerUids = nonEmptyLines(sections["OBSERVER_UIDS"]).mapNotNull { it.toIntOrNull() }.toSet()

        val pmLine = Regex("^package:(\\S+) uid:(\\d+)")
        val uidToPkg =
            sections["PM_LIST"]
                ?.lines()
                ?.mapNotNull { line ->
                    val m = pmLine.find(line) ?: return@mapNotNull null
                    m.groupValues[2].toInt() to m.groupValues[1]
                }?.toMap() ?: emptyMap()

        return TargetsSnapshot(
            kmodModuleInstalled = sections["KMOD_MODULE_DIR"]?.trim() == "1",
            zygiskModuleInstalled = sections["ZYGISK_MODULE_DIR"]?.trim() == "1",
            portsModuleInstalled = portsInstalled,
            kmodTargets = nonEmptyLines(sections["KMOD_TARGETS"]),
            zygiskTargets = nonEmptyLines(sections["ZYGISK_TARGETS"]),
            lsposedTargets = nonEmptyLines(sections["LSPOSED_TARGETS"]),
            hiddenPkgs = nonEmptyLines(sections["HIDDEN_PKGS"]),
            observerUids = observerUids,
            portsObservers = nonEmptyLines(sections["PORTS_OBSERVERS"]),
            uidToPkg = uidToPkg,
        )
    }
}
