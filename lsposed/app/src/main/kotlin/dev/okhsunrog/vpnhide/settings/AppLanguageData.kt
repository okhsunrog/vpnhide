package dev.okhsunrog.vpnhide.settings

internal fun appLanguageForTag(tag: String): AppLanguage =
    AppLanguage.entries.firstOrNull { it.tag.substringBefore('-') == tag.substringBefore(',').substringBefore('-').lowercase() }
        ?: AppLanguage.System
