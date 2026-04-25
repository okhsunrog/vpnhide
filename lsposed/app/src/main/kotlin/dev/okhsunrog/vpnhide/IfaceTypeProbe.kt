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
 * type under the same name.
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

    private val cache = ConcurrentHashMap<String, Boolean>()

    /** True if vpnhide should hide this iface from target apps. */
    fun shouldHide(name: String): Boolean {
        if (name.isEmpty()) return false
        if (IfaceLists.isNeverHide(name)) return false
        return isTunnel(name)
    }

    /** True if the kernel classifies this iface as a tunnel. */
    fun isTunnel(name: String): Boolean {
        if (name.isEmpty()) return false
        cache[name]?.let { return it }
        val v = readArphrd(name)?.let { TUNNEL_ARPHRDS.contains(it) } ?: false
        cache[name] = v
        return v
    }

    /** Raw `/sys/class/net/<name>/type` integer, or null on read failure. */
    fun readArphrd(name: String): Int? =
        try {
            File("/sys/class/net/$name/type").readText().trim().toIntOrNull()
        } catch (e: Exception) {
            null
        }
}
