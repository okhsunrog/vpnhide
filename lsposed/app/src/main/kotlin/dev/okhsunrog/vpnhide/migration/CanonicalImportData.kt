package dev.okhsunrog.vpnhide.migration

import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.canonicalConfigWithSelfTarget
import dev.okhsunrog.vpnhide.parseCanonicalConfig
import org.json.JSONArray
import org.json.JSONObject

/** 1.x JSON and current 2.x disk interchange share schema 1 (also read by LSPosed).
 * Reject damaged shapes before the tolerant hook reader can default them away.
 * Import never writes: the resulting typed model uses the ordinary core command.
 */
internal fun parseImportedCanonicalConfig(
    raw: String,
    selfPkg: String,
): CanonicalConfig? =
    runCatching {
        val root = JSONObject(raw)
        val version = if (root.has("version")) (root.get("version") as? Number)?.toString()?.toLongOrNull() else 1L
        require(version == 1L)
        checkFields(root, listOf("debug", "debugSwitch")) { it is Boolean }
        checkFields(root, listOf("apps", "settings")) { it is JSONObject }
        root.optJSONObject("settings")?.let(::validateSettings)
        root.optJSONObject("apps")?.let { apps ->
            apps.keys().forEach { pkg -> validateApp(apps.getJSONObject(pkg)) }
        }
        parseCanonicalConfig(raw)?.let { canonicalConfigWithSelfTarget(it, selfPkg) }
    }.getOrNull()

private fun checkFields(
    value: JSONObject,
    keys: List<String>,
    accepts: (Any) -> Boolean,
) {
    keys.forEach { key -> require(!value.has(key) || accepts(value.get(key))) { "Invalid $key" } }
}

private fun strings(value: Any): Boolean = value is JSONArray && (0 until value.length()).all { value.get(it) is String }

private fun role(value: Any): Boolean = value is Boolean || strings(value)

private fun validateSettings(settings: JSONObject) {
    checkFields(settings, listOf("rememberSuperkey", "autoHideVpnServices", "autoHideVpnName")) { it is Boolean }
    checkFields(settings, listOf("optionalFeatures", "autoHideExcludedPackages", "autoHiddenPackages"), ::strings)
}

private fun validateApp(app: JSONObject) {
    checkFields(app, listOf("java"), ::role)
    checkFields(app, listOf("ports", "hidden", "appHiding")) { it is Boolean }
    if (app.has("native")) {
        val native = app.get("native")
        if (native is JSONObject) {
            checkFields(native, listOf("enabled")) { it is Boolean }
            checkFields(native, listOf("kernel", "zygisk")) { it == JSONObject.NULL || strings(it) }
        } else {
            require(role(native))
        }
    }
    if (app.has("portPolicy") && !app.isNull("portPolicy")) validatePorts(app.getJSONObject("portPolicy"))
}

private fun validatePorts(policy: JSONObject) {
    checkFields(policy, listOf("mode")) { it is String }
    checkFields(policy, listOf("preset")) { it == JSONObject.NULL || it is String }
    val rules = policy.getJSONArray("rules")
    for (index in 0 until rules.length()) {
        val rule = rules.getJSONObject(index)
        val start = (rule.get("start") as? Number)?.toString()?.toIntOrNull()
        val end = if (rule.has("end")) (rule.get("end") as? Number)?.toString()?.toIntOrNull() else start
        require(start != null && end != null && start in 1..65535 && end in start..65535)
        checkFields(rule, listOf("protocol")) { it in listOf("tcp", "udp", "both") }
    }
}
