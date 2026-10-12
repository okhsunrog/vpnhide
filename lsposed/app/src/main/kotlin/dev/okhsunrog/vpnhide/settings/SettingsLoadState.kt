package dev.okhsunrog.vpnhide.settings

/** Null is genuinely unread, not an AppSettings default snapshot. */
internal data class SettingsLoadState(
    val settings: AppSettings? = null,
    val error: String? = null,
)
