package dev.okhsunrog.vpnhide

/** Staging never changes the lane metadata. Late staging has no authority to dispatch a mutation. */
internal fun buildRootMutationStageCommand(
    local: String,
    directory: String,
    digest: String,
    nonce: String,
): String {
    require(digest.matches(Regex("[0-9a-f]{64}")) && validRootIdentity(nonce))
    return buildContentAddressedExecutableStageCommand(
        source = local,
        target = "$directory/vhhelper-$digest",
        digest = digest,
        temporary = "$directory/stage-$nonce",
        directory = directory,
    )
}
