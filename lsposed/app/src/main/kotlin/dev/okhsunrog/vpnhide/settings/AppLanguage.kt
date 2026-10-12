package dev.okhsunrog.vpnhide.settings

/** Empty tag delegates to the system; native names remain readable in any UI language. */
internal enum class AppLanguage(
    val tag: String,
    val nativeName: String,
) {
    System("", ""),
    English("en", "English"),
    Russian("ru", "Русский"),
    Chinese("zh-CN", "简体中文"),
}
