package dev.okhsunrog.vpnhide.diagnostics

/** Only violations evaluated from available evidence can take precedence over errors. */
internal fun networkViewClean(
    hasViolation: Boolean,
    hasError: Boolean,
): Boolean? =
    when {
        hasViolation -> false
        hasError -> null
        else -> true
    }

internal data class CallbackProbeObservation(
    val network: Int,
    val interfaceName: String? = null,
    val leak: String? = null,
)

/** Publish a whole pair atomically; a later clean event never erases a seen leak. */
internal class CallbackProbeEvidence {
    private var capsNetwork: Int? = null
    private var linkNetwork: Int? = null
    private var linkInterface: String? = null
    private var completed: CallbackProbeObservation? = null

    @Synchronized
    fun capabilities(
        network: Int,
        leak: String?,
    ) {
        capsNetwork = network
        if (leak != null) completed = CallbackProbeObservation(network, leak = leak)
        completePair()
    }

    @Synchronized
    fun linkProperties(
        network: Int,
        interfaceName: String?,
    ) {
        linkNetwork = network
        linkInterface = interfaceName
        completePair()
    }

    @Synchronized
    fun snapshot(): CallbackProbeObservation? = completed

    private fun completePair() {
        val network = capsNetwork ?: return
        if (completed == null && linkNetwork == network) completed = CallbackProbeObservation(network, linkInterface)
    }
}
