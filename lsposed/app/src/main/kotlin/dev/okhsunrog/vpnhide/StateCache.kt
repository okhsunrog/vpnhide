package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.AppListCache
import dev.okhsunrog.vpnhide.picker.TargetsCache
import dev.okhsunrog.vpnhide.startup.StartupTrace
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * Shared base for the app-scoped, lazily-loaded caches (`DashboardCache`,
 * `TargetsCache`, `AppListCache`). They are structurally identical — a
 * nullable value, a loading flag, an error string, and a single in-flight
 * [Job] guarded so a tab switch doesn't re-run an expensive root-shell load.
 * Writing each by hand let them drift (one rethrew `CancellationException`,
 * another had no error state at all); funnelling them through one base keeps
 * that behaviour uniform.
 *
 * Concrete caches implement [load] (the actual work) and expose a typed
 * accessor over [value] plus their own `ensureLoaded` / `refresh` signatures
 * that stash any inputs `load` needs before delegating to [ensure] /
 * [forceRefresh].
 *
 * [traceName] is used to emit `<name>_start` / `<name>_done` / `<name>_failed`
 * StartupTrace events (see scripts/measure-startup.py for the consumers).
 */
internal abstract class StateCache<T>(
    private val traceName: String,
    private val logTag: String,
) {
    private val _value = MutableStateFlow<T?>(null)
    val value: StateFlow<T?> = _value.asStateFlow()

    private val _loading = MutableStateFlow(false)
    val loading: StateFlow<Boolean> = _loading.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    private var inflight: Job? = null

    /** Produce a fresh value. [force] is true on an explicit user refresh,
     * false on the initial lazy load — concrete caches use it to decide
     * whether to bust the shared root snapshot. */
    protected abstract suspend fun load(force: Boolean): T

    /** Kick off the initial load unless a value/error already exists or a
     * load is in flight. Idempotent — safe to call on every composition. */
    protected fun ensure(scope: CoroutineScope) {
        if (_value.value != null || _error.value != null || inflight?.isActive == true) return
        inflight = scope.launch { reload(force = false) }
    }

    /** Cancel any in-flight load and start a forced reload. */
    protected fun forceRefresh(scope: CoroutineScope) {
        inflight?.cancel()
        _error.value = null
        inflight = scope.launch { reload(force = true) }
    }

    /**
     * Reload the value in place and await it: recompute via [load] and swap the result in
     * one assignment, so subscribers keep the current value until the fresh one is ready —
     * no null gap, hence no UI flicker. For suspend callers (a config write) that must
     * leave the cache coherent before returning. On failure the stale value is kept and
     * the error surfaced. Contrast [invalidate], which drops to null and defers the reload.
     *
     * @param force forwarded to [load]; pass false to reuse an already-refreshed upstream
     *   (e.g. the root snapshot) instead of busting it again.
     */
    suspend fun refreshInPlace(force: Boolean = true) {
        inflight?.cancel()
        inflight = null
        reload(force)
    }

    /**
     * True while the cache has never held anything — no value, no failed load.
     *
     * A pristine cache has nothing that can go stale, so a config write can skip
     * it: the next [ensure] loads it fresh anyway. Skipping also matters because
     * [refreshInPlace] bypasses the concrete cache's `ensureLoaded`, so calling it
     * first would run [load] without the inputs that method stashes — the load
     * fails, [reload] records the error, and [ensure] then early-returns on that
     * error forever. Kept in the error case on purpose: that one is worth retrying.
     */
    val pristine: Boolean
        get() = _value.value == null && _error.value == null

    /** Drop the cached value/error so the next [ensure] reloads. */
    open fun invalidate() {
        _value.value = null
        _error.value = null
    }

    private suspend fun reload(force: Boolean) {
        _loading.value = true
        try {
            StartupTrace.mark("${traceName}_start")
            _value.value = load(force)
            _error.value = null
            StartupTrace.mark("${traceName}_done")
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            StartupTrace.mark("${traceName}_failed")
            _error.value = e.message ?: e.javaClass.simpleName
            VpnHideLog.w(logTag, "$traceName reload failed: ${e.message}", e)
        } finally {
            _loading.value = false
        }
    }
}
