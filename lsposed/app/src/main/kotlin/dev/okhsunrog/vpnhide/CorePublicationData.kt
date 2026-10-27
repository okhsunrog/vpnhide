package dev.okhsunrog.vpnhide

/** One owner fences every publication and transport failure, including the long poll. */
internal data class CorePublicationData(
    val update: CoreSessionUpdate = CoreSessionUpdate(),
    val latestSuccessfulRequest: Long = 0,
    val successfulStateGeneration: Long = 0,
    val failureGeneration: Long = 0,
    val latestFailedRequest: Long = 0,
)

internal fun acceptCorePublication(
    order: CorePublicationData,
    connectionGeneration: Long,
    request: Long,
    incoming: CoreStateView,
): CorePublicationData {
    val previous = order.update
    if (connectionGeneration < previous.connectionGeneration ||
        (connectionGeneration == order.failureGeneration && request < order.latestFailedRequest)
    ) {
        return order
    }
    if (connectionGeneration == order.successfulStateGeneration && previous.state != null) {
        // A connection is tied to one server epoch. Even an unseen late epoch cannot replace it.
        if (incoming.epoch != previous.state.epoch || incoming.revision < previous.state.revision) return order
    }
    return CorePublicationData(
        CoreSessionUpdate(incoming, current = true, sequence = previous.sequence + 1, connectionGeneration = connectionGeneration),
        if (connectionGeneration == order.successfulStateGeneration) maxOf(request, order.latestSuccessfulRequest) else request,
        connectionGeneration,
        order.failureGeneration,
        order.latestFailedRequest,
    )
}

internal fun failCorePublication(
    order: CorePublicationData,
    connectionGeneration: Long,
    request: Long,
    protocolSkew: Boolean,
): CorePublicationData {
    if (connectionGeneration < order.update.connectionGeneration ||
        (connectionGeneration == order.successfulStateGeneration && request < order.latestSuccessfulRequest) ||
        (connectionGeneration == order.failureGeneration && request < order.latestFailedRequest)
    ) {
        return order
    }
    return order.copy(
        update =
            order.update.copy(
                current = false,
                protocolSkew = protocolSkew,
                sequence = order.update.sequence + 1,
                connectionGeneration = connectionGeneration,
            ),
        failureGeneration = connectionGeneration,
        latestFailedRequest = request,
    )
}
