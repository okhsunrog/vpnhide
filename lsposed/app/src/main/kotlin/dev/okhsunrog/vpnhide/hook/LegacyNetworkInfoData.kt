package dev.okhsunrog.vpnhide.hook

/**
 * A legacy type query describes support/state, not the existence of a Network
 * handle. Keep absent/unchanged replies by identity, including OEM nulls.
 */
internal fun <T> normalizeLegacyVpnInfo(
    original: T?,
    requestedType: Int?,
    typeOf: (T) -> Int,
    isInactive: (T) -> Boolean,
    absentInfo: () -> T,
): T? {
    if (original == null) return null
    val type = typeOf(original)
    if (requestedType != 17 && type != 17) return original
    if (type == 17 && isInactive(original)) return original
    return absentInfo()
}
