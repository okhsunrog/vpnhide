package dev.okhsunrog.vpnhide.hook

/** Try modern attribution, Android 11 package attribution, then the legacy accessor.
 * A successful null means the network disappeared; only a missing method permits fallback.
 * Invocation failures propagate to the hook boundary instead of retrying with less attribution.
 */
internal fun <T> readNetworkCapabilitiesCompat(
    network: Any,
    call: (Array<Any?>) -> T?,
): T? {
    val modern = arrayOf(network, "android", null)
    for (count in modern.size downTo 1) {
        try {
            return call(modern.copyOf(count))
        } catch (_: NoSuchMethodError) {
            // Xposed reports an absent signature as an Error.
        } catch (_: NoSuchMethodException) {
            // Plain reflection reports an absent signature as an Exception.
        }
    }
    return null
}
