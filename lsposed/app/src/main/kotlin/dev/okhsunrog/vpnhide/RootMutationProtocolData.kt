package dev.okhsunrog.vpnhide

import org.json.JSONObject
import org.json.JSONTokener

private val rootIdentityPattern = Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")

internal fun validRootIdentity(value: String): Boolean = rootIdentityPattern.matches(value)

internal fun rootMutationScriptInput(command: String): ByteArray {
    val payload = command.toByteArray(Charsets.UTF_8)
    require(payload.isNotEmpty() && payload.size <= 2 * 1024 * 1024 && '\u0000' !in command)
    return "vpnhide-script 1 ${payload.size}\n".toByteArray(Charsets.UTF_8) + payload
}

/** Strict version/identity/shape checks: malformed output never becomes a successful receipt. */
internal fun parseRootMutationReply(raw: String): RootMutationReply =
    try {
        val tokener = JSONTokener(raw)
        val json = tokener.nextValue() as JSONObject
        require(tokener.nextClean() == '\u0000')
        require(json.get("version") == 1)
        when (json.getString("status")) {
            "busy" -> {
                RootMutationReply.Busy
            }

            "unavailable" -> {
                RootMutationReply.Unavailable
            }

            "ok", "rejected" -> {
                val boot = json.getString("boot").also { require(validRootIdentity(it)) }
                val receipt = parseRootReceipt(json.getJSONObject("state"))
                val canonical = parseRootCanonical(json)
                require(receipt.boot != boot || receipt.status != RootReceiptStatus.Running || canonical == RootCanonicalRead.Unavailable)
                RootMutationReply.Observed(json.getString("status") == "ok", RootMutationSnapshot(boot, receipt, canonical))
            }

            else -> {
                RootMutationReply.Unavailable
            }
        }
    } catch (_: Exception) {
        RootMutationReply.Unavailable
    }

private fun parseRootReceipt(json: JSONObject): RootMutationReceipt {
    require(json.get("version") == 1)
    require(json.has("session") && json.has("exit_code"))
    val status =
        when (json.getString("status")) {
            "idle" -> RootReceiptStatus.Idle
            "running" -> RootReceiptStatus.Running
            "finished" -> RootReceiptStatus.Finished
            "not_started" -> RootReceiptStatus.NotStarted
            else -> error("invalid status")
        }
    val sequence = rootInteger(json, "sequence")
    val session = if (json.isNull("session")) null else json.getString("session").also { require(validRootIdentity(it)) }
    val exit = if (json.isNull("exit_code")) null else rootInteger(json, "exit_code").also { require(it <= 255) }.toInt()
    val descendantFailed = json.get("descendant_failed") as Boolean
    require((status == RootReceiptStatus.Idle) == (sequence == 0L))
    require(session != null || status == RootReceiptStatus.Idle)
    require((status == RootReceiptStatus.Finished) == (exit != null))
    require(!descendantFailed || status == RootReceiptStatus.Finished)
    return RootMutationReceipt(
        rootInteger(json, "revision"),
        json.getString("boot").also { require(validRootIdentity(it)) },
        session,
        sequence,
        status,
        exit,
        descendantFailed,
    )
}

private fun rootInteger(
    json: JSONObject,
    name: String,
): Long {
    val number = json.get(name)
    require(number is Int || number is Long)
    return (number as Number).toLong().also { require(it in 0 until Long.MAX_VALUE) }
}

private fun parseRootCanonical(json: JSONObject): RootCanonicalRead =
    when (json.getString("config_status")) {
        "readable" -> {
            val raw = json.getString("config")
            runCatching { parseCanonicalConfig(raw) }.getOrNull()?.let { RootCanonicalRead.Available(it) } ?: RootCanonicalRead.Invalid
        }

        "missing" -> {
            require(json.isNull("config"))
            RootCanonicalRead.Missing
        }

        "unavailable" -> {
            require(json.isNull("config"))
            RootCanonicalRead.Unavailable
        }

        else -> {
            error("invalid config status")
        }
    }
