package dev.okhsunrog.vpnhide

/** Staging never changes the lane metadata. Late staging has no authority to dispatch a mutation. */
internal fun buildRootMutationStageCommand(
    local: String,
    directory: String,
    digest: String,
    nonce: String,
): String {
    require(digest.matches(Regex("[0-9a-f]{64}")) && validRootIdentity(nonce))
    val root = shellQuote(directory)
    val target = shellQuote("$directory/vhhelper-$digest")
    val temporary = shellQuote("$directory/stage-$nonce")
    return "umask 077; mkdir -p $root && chmod 700 $root && " +
        "{ if [ ! -f $target ]; then " +
        "cp ${shellQuote(local)} $temporary && chmod 700 $temporary && " +
        "{ ln $temporary $target 2>/dev/null || [ -f $target ]; }; " +
        "fi; } && rm -f $temporary && " +
        "[ \"\$(sha256sum $target | cut -d ' ' -f 1)\" = '$digest' ]"
}
