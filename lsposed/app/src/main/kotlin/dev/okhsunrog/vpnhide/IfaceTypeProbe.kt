package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.IfaceLists
import java.io.File
import java.util.concurrent.ConcurrentHashMap

/**
 * Decides whether `vpnhide` should hide an interface, by asking the
 * kernel directly (via `/sys/class/net/<name>/type`) instead of
 * guessing from the name.
 *
 * Why: a hostile VPN client can rename its `tun0` to `if33` (issue
 * #86), so name-based detection is bypassable. The ARPHRD type stored
 * in the kernel is set at netdev registration and cannot be forged
 * from an unprivileged process.
 *
 * Two tunnel-class interfaces must NOT be hidden — CLAT shadow
 * (`v4-rmnet0`, ...) and the Thread border router (`thread-wpan`).
 * Those go through `IfaceLists.isNeverHide` (generated from
 * `data/interfaces.toml`).
 *
 * Per-name cache: read once on first sighting, keep forever. Names
 * are stable in practice — interfaces come and go but rarely change
 * type under the same name. Only successful reads land in the cache,
 * so a direct-read failure (e.g. SELinux denial in app context)
 * doesn't poison subsequent root-fallback lookups.
 *
 * Two read paths share the same matching/cache logic via [shouldHideWith]:
 *   - [shouldHide]: direct File read — for hook context
 *     (system_server, hook processes) where sysfs is reachable.
 *   - [shouldHideViaRoot]: root via `suExec` — for app context where
 *     SELinux blocks untrusted_app's access to
 *     `/sys/class/net/<name>/type`. The diagnostic UI specifically
 *     needs this because the app hides VPN ifaces from itself for
 *     self-test, so it has to bypass its own filter via root to learn
 *     the ground truth.
 */
internal object IfaceTypeProbe {
    /** ARPHRD_* values from `<linux/if_arp.h>` that mean "tunnel". */
    private val TUNNEL_ARPHRDS =
        setOf(
            0xFFFE, // ARPHRD_NONE  (TUN)
            512, // ARPHRD_PPP
            768, // ARPHRD_TUNNEL  (IPIP)
            769, // ARPHRD_TUNNEL6 (IPIP6)
            776, // ARPHRD_SIT     (IPv6 in IPv4)
            778, // ARPHRD_IPGRE
        )

    /** ARPHRD value per name. Only successful reads land here. */
    private val arphrdCache = ConcurrentHashMap<String, Int>()

    /** Hook context: read sysfs directly. */
    fun shouldHide(name: String): Boolean = shouldHideWith(name, ::readArphrd)

    /** App context: read sysfs via `su` (SELinux blocks the direct path). */
    fun shouldHideViaRoot(name: String): Boolean = shouldHideWith(name, ::readArphrdViaRoot)

    /**
     * Batch-fill the cache for every netdev currently in `/sys/class/net/`
     * via a single `su` invocation. Cheap (one shell roundtrip vs N), and
     * makes subsequent [shouldHideViaRoot] / [shouldHide] calls hit the
     * cache. Returns the number of entries actually populated.
     */
    fun prefetchAllViaRoot(): Int {
        val (exit, out) =
            suExec(
                """
                for f in /sys/class/net/*; do
                    [ -f "${'$'}f/type" ] && echo "${'$'}{f##*/} ${'$'}(cat ${'$'}f/type 2>/dev/null)"
                done
                """.trimIndent(),
            )
        if (exit != 0) return 0
        var n = 0
        for (line in out.lines()) {
            val parts = line.trim().split(Regex("\\s+"))
            if (parts.size != 2) continue
            val arphrd = parts[1].toIntOrNull() ?: continue
            arphrdCache[parts[0]] = arphrd
            n++
        }
        return n
    }

    private inline fun shouldHideWith(
        name: String,
        reader: (String) -> Int?,
    ): Boolean {
        if (name.isEmpty() || IfaceLists.isNeverHide(name)) return false
        val arphrd =
            arphrdCache[name]
                ?: reader(name)?.also { arphrdCache[name] = it }
                ?: return false
        return arphrd in TUNNEL_ARPHRDS
    }

    private fun readArphrd(name: String): Int? =
        try {
            File("/sys/class/net/$name/type").readText().trim().toIntOrNull()
        } catch (e: Exception) {
            null
        }

    private fun readArphrdViaRoot(name: String): Int? {
        val (exit, out) = suExec("cat /sys/class/net/$name/type 2>/dev/null")
        return if (exit == 0) out.trim().toIntOrNull() else null
    }
}
