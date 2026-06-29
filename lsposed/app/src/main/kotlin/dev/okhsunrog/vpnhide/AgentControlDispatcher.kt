package dev.okhsunrog.vpnhide

import android.content.Context
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject

@Suppress("LargeClass")
internal object AgentControlDispatcher {
    private val functions: List<BridgeFunction> =
        listOf(
            function<RefreshArgs, AgentDashboardState>(
                name = "getDashboardState",
                description = "Return the current dashboard state. Set refresh to true to force a fresh root snapshot.",
                inputSchema = schema(optional("refresh", booleanSchema())),
            ) { context, args ->
                AgentControl.getDashboardState(context, args.refresh)
            },
            function<EmptyArgs, AgentDiagnosticsReport>(
                name = "runFullDiagnostics",
                description = "Run the full diagnostics suite and return every check shown in Detailed diagnostics.",
                inputSchema = schema(),
            ) { context, _ ->
                AgentControl.runFullDiagnostics(context)
            },
            function<RefreshArgs, AgentStatisticsState>(
                name = "getStatisticsState",
                description = "Return the Statistics tab state, including backend rows and per-app probe rollups.",
                inputSchema = schema(optional("refresh", booleanSchema())),
            ) { context, args ->
                AgentControl.getStatisticsState(context, args.refresh)
            },
            function<RefreshArgs, AgentStatisticsCaptureBaseline>(
                name = "getStatisticsCaptureBaseline",
                description = "Take a cumulative-counter baseline for the Statistics capture-session flow.",
                inputSchema = schema(optional("refresh", booleanSchema())),
            ) { context, args ->
                AgentControl.getStatisticsCaptureBaseline(context, args.refresh)
            },
            function<CaptureDiffArgs, AgentStatisticsCaptureDiff>(
                name = "getStatisticsCaptureDiff",
                description = "Return probes that happened since a baseline from getStatisticsCaptureBaseline.",
                inputSchema = schema(required("baseline", captureBaselineSchema()), optional("refresh", booleanSchema())),
            ) { context, args ->
                AgentControl.getStatisticsCaptureDiff(context, args.baseline, args.refresh)
            },
            function<RefreshArgs, AgentProtectionState>(
                name = "getProtectionState",
                description = "Return the Protection tab canonical config and configured package summary.",
                inputSchema = schema(optional("refresh", booleanSchema())),
            ) { context, args ->
                AgentControl.getProtectionState(context, args.refresh)
            },
            function<ListInstalledAppsArgs, List<AgentInstalledApp>>(
                name = "listInstalledApps",
                description = "List installed apps in the same shape used by the Protection picker.",
                inputSchema =
                    schema(
                        optional("includeSystem", booleanSchema()),
                        optional("configuredOnly", booleanSchema()),
                        optional("refresh", booleanSchema()),
                    ),
            ) { context, args ->
                AgentControl.listInstalledApps(
                    context = context,
                    includeSystem = args.includeSystem,
                    configuredOnly = args.configuredOnly,
                    refresh = args.refresh,
                )
            },
            function<EmptyArgs, String>(
                name = "exportCanonicalConfig",
                description = "Export the canonical JSON config used by Settings backup/export.",
                inputSchema = schema(),
            ) { context, _ ->
                AgentControl.exportCanonicalConfig(context)
            },
            function<ImportCanonicalConfigArgs, AgentMutationResult>(
                name = "importCanonicalConfig",
                description = "Import canonical JSON and immediately activate native/ports runtime state.",
                inputSchema = schema(required("json", stringSchema())),
            ) { context, args ->
                AgentControl.importCanonicalConfig(context, args.json)
            },
            function<SetAppProtectionArgs, AgentMutationResult>(
                name = "setAppProtection",
                description = "Set high-level protection roles for one package. Null or omitted role arguments leave that role unchanged.",
                inputSchema =
                    schema(
                        required("packageName", stringSchema()),
                        optional("java", booleanSchema()),
                        optional("native", booleanSchema()),
                        optional("appHiding", booleanSchema()),
                        optional("ports", booleanSchema()),
                        optional("hidden", booleanSchema()),
                    ),
            ) { context, args ->
                AgentControl.setAppProtection(
                    context = context,
                    packageName = args.packageName,
                    java = args.java,
                    native = args.native,
                    appHiding = args.appHiding,
                    ports = args.ports,
                    hidden = args.hidden,
                )
            },
            function<SetJavaHooksArgs, AgentMutationResult>(
                name = "setJavaHooks",
                description = "Set exact Java hook selection for one package. An empty hookIds list disables Java for that package.",
                inputSchema = schema(required("packageName", stringSchema()), required("hookIds", stringArraySchema())),
            ) { context, args ->
                AgentControl.setJavaHooks(context, args.packageName, args.hookIds)
            },
            function<SetNativeHooksArgs, AgentMutationResult>(
                name = "setNativeHooks",
                description = "Set exact native hook selection for one package and native family. An empty hookIds list disables Native.",
                inputSchema =
                    schema(
                        required("packageName", stringSchema()),
                        optional("family", stringSchema()),
                        required("hookIds", stringArraySchema()),
                    ),
            ) { context, args ->
                AgentControl.setNativeHooks(context, args.packageName, args.family, args.hookIds)
            },
            function<SetPortPolicyArgs, AgentMutationResult>(
                name = "setPortPolicy",
                description = "Set localhost port policy for one package. Use mode all to block every localhost port.",
                inputSchema =
                    schema(
                        required("packageName", stringSchema()),
                        required("mode", stringSchema()),
                        optional("preset", stringSchema()),
                        optional("rules", arraySchema(portRuleSchema())),
                    ),
            ) { context, args ->
                AgentControl.setPortPolicy(context, args.packageName, args.mode, args.preset, args.rules)
            },
            function<SetAutoHideSettingsArgs, AgentMutationResult>(
                name = "setAutoHideSettings",
                description = "Update app-hiding auto-detection heuristics.",
                inputSchema = schema(optional("autoHideVpnServices", booleanSchema()), optional("autoHideVpnName", booleanSchema())),
            ) { context, args ->
                AgentControl.setAutoHideSettings(context, args.autoHideVpnServices, args.autoHideVpnName)
            },
            function<PackageNamesArgs, AgentMutationResult>(
                name = "setManualHiddenPackages",
                description = "Replace manual hidden-package selection. Auto-hidden packages are preserved.",
                inputSchema = schema(required("packageNames", stringArraySchema())),
            ) { context, args ->
                AgentControl.setManualHiddenPackages(context, args.packageNames)
            },
            function<PackageNamesArgs, AgentMutationResult>(
                name = "removeConfiguredPackages",
                description = "Remove stale configured packages from canonical config.",
                inputSchema = schema(required("packageNames", stringArraySchema())),
            ) { context, args ->
                AgentControl.removeConfiguredPackages(context, args.packageNames)
            },
            function<SetDebugLoggingArgs, AgentMutationResult>(
                name = "setDebugLogging",
                description = "Toggle VPN Hide debug logging and propagate it to runtime sinks.",
                inputSchema = schema(required("enabled", booleanSchema())),
            ) { context, args ->
                AgentControl.setDebugLogging(context, args.enabled)
            },
            function<EmptyArgs, AgentMutationResult>(
                name = "activateConfig",
                description = "Re-run native and ports activators for the current canonical config.",
                inputSchema = schema(),
            ) { context, _ ->
                AgentControl.activateConfig(context)
            },
        )

    val functionSpecs: List<AgentBridgeFunctionSpec> =
        functions.map { AgentBridgeFunctionSpec(it.name, it.description, it.inputSchema) }

    fun functionsJson(): String = AgentBridgeJson.encodeToString(AgentBridgeFunctionsResponse(functionSpecs))

    suspend fun call(
        context: Context,
        requestJson: String,
    ): String {
        val request = AgentBridgeJson.decodeFromString<AgentBridgeCall>(requestJson)
        return call(context, request.fn, request.args)
    }

    suspend fun call(
        context: Context,
        name: String,
        args: JsonObject,
    ): String {
        val function = functions.firstOrNull { it.name == name } ?: throw IllegalArgumentException("Unknown function: $name")
        return function.call(context.applicationContext, args)
    }
}

private data class BridgeFunction(
    val name: String,
    val description: String,
    val inputSchema: JsonObject,
    val call: suspend (Context, JsonObject) -> String,
)

private inline fun <reified Args, reified Result> function(
    name: String,
    description: String,
    inputSchema: JsonObject,
    crossinline block: suspend (Context, Args) -> Result,
): BridgeFunction =
    BridgeFunction(name, description, inputSchema) { context, argsJson ->
        val args = AgentBridgeJson.decodeFromJsonElement<Args>(argsJson)
        AgentBridgeJson.encodeToString(block(context, args))
    }

@Serializable
private class EmptyArgs

@Serializable
private data class RefreshArgs(
    val refresh: Boolean? = null,
)

@Serializable
private data class CaptureDiffArgs(
    val baseline: AgentStatisticsCaptureBaseline,
    val refresh: Boolean? = null,
)

@Serializable
private data class ListInstalledAppsArgs(
    val includeSystem: Boolean? = null,
    val configuredOnly: Boolean? = null,
    val refresh: Boolean? = null,
)

@Serializable
private data class ImportCanonicalConfigArgs(
    val json: String,
)

@Serializable
private data class SetAppProtectionArgs(
    val packageName: String,
    val java: Boolean? = null,
    val native: Boolean? = null,
    val appHiding: Boolean? = null,
    val ports: Boolean? = null,
    val hidden: Boolean? = null,
)

@Serializable
private data class SetJavaHooksArgs(
    val packageName: String,
    val hookIds: List<String>,
)

@Serializable
private data class SetNativeHooksArgs(
    val packageName: String,
    val family: String? = null,
    val hookIds: List<String>,
)

@Serializable
private data class SetPortPolicyArgs(
    val packageName: String,
    val mode: String,
    val preset: String? = null,
    val rules: List<AgentPortRule>? = null,
)

@Serializable
private data class SetAutoHideSettingsArgs(
    val autoHideVpnServices: Boolean? = null,
    val autoHideVpnName: Boolean? = null,
)

@Serializable
private data class PackageNamesArgs(
    val packageNames: List<String>,
)

@Serializable
private data class SetDebugLoggingArgs(
    val enabled: Boolean,
)

private data class SchemaProp(
    val name: String,
    val schema: JsonObject,
    val required: Boolean,
)

private fun required(
    name: String,
    schema: JsonObject,
) = SchemaProp(name, schema, required = true)

private fun optional(
    name: String,
    schema: JsonObject,
) = SchemaProp(name, schema, required = false)

private fun schema(vararg props: SchemaProp): JsonObject =
    buildJsonObject {
        put("type", "object")
        putJsonObject("properties") {
            props.forEach { put(it.name, it.schema) }
        }
        putJsonArray("required") {
            props.filter(SchemaProp::required).forEach { add(JsonPrimitive(it.name)) }
        }
        put("additionalProperties", false)
    }

private fun stringSchema(): JsonObject = buildJsonObject { put("type", "string") }

private fun booleanSchema(): JsonObject = buildJsonObject { put("type", "boolean") }

private fun integerSchema(): JsonObject = buildJsonObject { put("type", "integer") }

private fun stringArraySchema(): JsonObject = arraySchema(stringSchema())

private fun arraySchema(itemSchema: JsonObject): JsonObject =
    buildJsonObject {
        put("type", "array")
        put("items", itemSchema)
    }

private fun portRuleSchema(): JsonObject =
    schema(
        required("protocol", stringSchema()),
        required("start", integerSchema()),
        required("end", integerSchema()),
    )

private fun captureBaselineSchema(): JsonObject =
    schema(
        required(
            "counters",
            arraySchema(
                schema(
                    required("uid", integerSchema()),
                    required("hookId", integerSchema()),
                    required("count", integerSchema()),
                ),
            ),
        ),
    )
