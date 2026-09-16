package dev.okhsunrog.vpnhide

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RootMutationDataTest {
    @Test
    fun `script envelope counts UTF8 bytes rather than characters`() {
        val command = "echo привет"
        assertEquals("vpnhide-script 1 ${command.toByteArray().size}\n$command", rootMutationScriptInput(command).toString(Charsets.UTF_8))
    }

    private val boot = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
    private val session = RootMutationSession(boot, "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb")
    private val base = CanonicalConfig()
    private val candidate = base.copy(debug = true)

    private fun reply(config: CanonicalConfig = candidate): JSONObject =
        JSONObject(
            """{"version":1,"status":"ok","boot":"$boot","state":{
        "version":1,"revision":3,"boot":"$boot","session":"${session.id}","sequence":1,
        "status":"finished","exit_code":0,"descendant_failed":false},"config_status":"readable"}""",
        ).put("config", canonicalConfigJson(config))

    private fun outcome(
        json: JSONObject,
        phase: ConfigPhase = ConfigPhase.Persist,
    ): PhaseOutcome = rootMutationPhaseOutcome(parseRootMutationReply(json.toString()), session, 1, phase, base, candidate)

    @Test
    fun `capacity evidence only accepts bounded consistent counts on terminal receipt`() {
        val json = reply().apply { getJSONObject("state").put("native_capacity", JSONObject("""{"total":10,"cap":8,"dropped":2}""")) }
        val parsed = parseRootMutationReply(json.toString()) as RootMutationReply.Observed
        assertEquals(
            2,
            parsed.snapshot.receipt.nativeCapacity
                ?.dropped,
        )
        for (bad in listOf(
            """{"total":10,"cap":0,"dropped":10}""",
            """{"total":10,"cap":8,"dropped":1}""",
            """{"total":2147483648,"cap":8,"dropped":2147483640}""",
            """{"total":10,"cap":8,"dropped":2,"text":"secret"}""",
        )) {
            json.getJSONObject("state").put("native_capacity", JSONObject(bad))
            assertEquals(RootMutationReply.Unavailable, parseRootMutationReply(json.toString()))
        }
    }

    @Test
    fun `atomic persistence depends on actual readback even after command failure or success`() {
        val partial = reply().apply { getJSONObject("state").put("exit_code", 7) }
        assertEquals(PhaseOutcome.Confirmed, outcome(partial))
        assertEquals(PhaseOutcome.FailedKnown, outcome(reply(base)))
        assertEquals(PhaseOutcome.Unknown, outcome(reply(base.copy(debugSwitch = true))))
        assertEquals(PhaseOutcome.FailedKnown, outcome(partial, ConfigPhase.Native))
    }

    @Test
    fun `matching config and unlocked process cannot override an unfinished receipt`() {
        val json =
            reply().apply {
                getJSONObject("state").put("status", "running").put("exit_code", JSONObject.NULL)
            }
        assertEquals(RootMutationReply.Unavailable, parseRootMutationReply(json.toString()))
        json.put("config_status", "unavailable").put("config", JSONObject.NULL)
        val read = parseRootMutationReply(json.toString()) as RootMutationReply.Observed
        assertFalse(read.snapshot.quiescent)
        assertEquals(PhaseOutcome.Unknown, outcome(json))
    }

    @Test
    fun `obsolete identity or boot never acknowledges a phase`() {
        for (field in listOf("boot", "session", "sequence")) {
            val json = reply()
            json.getJSONObject("state").put(field, if (field == "sequence") 2 else "cccccccc-cccc-cccc-cccc-cccccccccccc")
            assertEquals(PhaseOutcome.Unknown, outcome(json))
        }
        assertEquals(PhaseOutcome.Unknown, outcome(reply().put("status", "rejected")))
    }

    @Test
    fun `fenced launch is a known non-execution and descendant failure prevents success`() {
        val fenced = reply().apply { getJSONObject("state").put("status", "not_started").put("exit_code", JSONObject.NULL) }
        assertEquals(PhaseOutcome.FailedKnown, outcome(fenced))
        val childFailed = reply().apply { getJSONObject("state").put("descendant_failed", true) }
        assertEquals(PhaseOutcome.FailedKnown, outcome(childFailed, ConfigPhase.Native))
    }

    @Test
    fun `missing invalid and unreadable canonical observations stay distinct`() {
        for ((status, expected) in listOf("missing" to RootCanonicalRead.Missing, "unavailable" to RootCanonicalRead.Unavailable)) {
            val json = reply().put("config_status", status).put("config", JSONObject.NULL)
            val parsed = parseRootMutationReply(json.toString()) as RootMutationReply.Observed
            assertEquals(expected, parsed.snapshot.canonical)
            assertEquals(PhaseOutcome.Unknown, outcome(json))
        }
        val invalid = parseRootMutationReply(reply().put("config", "invalid").toString()) as RootMutationReply.Observed
        assertEquals(RootCanonicalRead.Invalid, invalid.snapshot.canonical)
    }

    @Test
    fun `malformed protocol cannot masquerade as an acknowledgement`() {
        val fractional = reply().apply { getJSONObject("state").put("sequence", 1.5) }
        val coerced = reply().apply { getJSONObject("state").put("sequence", "1") }
        for (raw in listOf(
            "",
            "{}",
            reply().put("version", 2).toString(),
            fractional.toString(),
            coerced.toString(),
            reply().toString() + "{}",
        )) {
            assertEquals(RootMutationReply.Unavailable, parseRootMutationReply(raw))
        }
        assertTrue(validRootIdentity(boot))
        assertFalse(validRootIdentity("'; command"))
    }
}
