package dev.okhsunrog.vpnhide.help

/** Shared by contextual entry points and links inside the guide. */
internal fun parseHelpDestination(value: String?): HelpDestination =
    HelpDestination(
        articleId = value?.substringBefore('#')?.ifEmpty { null },
        anchor = value?.substringAfter('#', "")?.ifEmpty { null },
    )
