package dev.okhsunrog.vpnhide

internal interface RootMutationClient {
    fun inspect(): RootMutationReply

    fun open(
        expected: RootMutationSnapshot,
        sessionId: String,
    ): RootMutationReply

    /** inspect + open under one lock and one privileged round trip; rejection still carries the receipt. */
    fun adopt(sessionId: String): RootMutationReply

    fun execute(
        session: RootMutationSession,
        sequence: Long,
        command: String,
    ): RootMutationReply

    fun recover(
        session: RootMutationSession,
        sequence: Long,
    ): RootMutationReply
}

/**
 * One instance belongs to the process-owned config coordinator. Each phase has a session + sequence;
 * coordinator recovery calls recover(), never execute() again. Commands remain private effect inputs.
 */
internal class RootMutationTransport(
    private val executable: String,
    private val directory: String,
    private val canonicalPath: String = CANONICAL_CONFIG_FILE,
    private val runner: RootProcessRunner = RootProcessRunner(),
) : RootMutationClient {
    override fun inspect(): RootMutationReply = call(listOf("inspect"))

    override fun open(
        expected: RootMutationSnapshot,
        sessionId: String,
    ): RootMutationReply {
        require(validRootIdentity(expected.boot) && validRootIdentity(sessionId))
        return call(listOf("open", expected.boot, expected.receipt.revision.toString(), sessionId))
    }

    override fun adopt(sessionId: String): RootMutationReply {
        require(validRootIdentity(sessionId))
        return call(listOf("adopt", sessionId))
    }

    override fun execute(
        session: RootMutationSession,
        sequence: Long,
        command: String,
    ): RootMutationReply = call(phaseArguments("run", session, sequence), rootMutationScriptInput(command))

    /** Readback plus a metadata fence for a launch still waiting in su; never replays a config effect. */
    override fun recover(
        session: RootMutationSession,
        sequence: Long,
    ): RootMutationReply = call(phaseArguments("recover", session, sequence))

    private fun phaseArguments(
        verb: String,
        session: RootMutationSession,
        sequence: Long,
    ): List<String> {
        require(validRootIdentity(session.boot) && validRootIdentity(session.id) && sequence in 1 until Long.MAX_VALUE)
        return listOf(verb, session.boot, session.id, sequence.toString())
    }

    private fun call(
        arguments: List<String>,
        script: ByteArray = byteArrayOf(),
    ): RootMutationReply {
        require(script.size <= 2 * 1024 * 1024 + 64)
        // -c contains paths and receipt IDs only; scripts/secrets travel on stdin and never through suExec logging.
        val command =
            (listOf(executable, "mutation", directory, canonicalPath) + arguments)
                .joinToString(" ", transform = ::shellQuote)
        return when (val result = runner.run(listOf("su", "-c", command), script)) {
            is RootProcessResult.Completed -> parseRootMutationReply(result.output)
            RootProcessResult.Uncertain -> RootMutationReply.Unavailable
        }
    }
}
