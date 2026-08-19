package dev.okhsunrog.vpnhide

internal const val PM_USERS_STATUS_PREFIX = "__VPNHIDE_PM_USERS_STATUS__:"
internal const val PM_USER_BEGIN_PREFIX = "__VPNHIDE_PM_USER_BEGIN__:"
internal const val PM_USER_END_PREFIX = "__VPNHIDE_PM_USER_END__:"
private const val PACKAGE_UIDS_PER_USER = 100_000

internal data class PackageInventoryEntry(
    val apkPath: String?,
    val uidsByUser: Map<Int, List<Int>>,
) {
    val userIds: List<Int> get() = uidsByUser.keys.sorted()
    val uids: List<Int> get() =
        uidsByUser.values
            .flatten()
            .distinct()
            .sorted()
}

internal data class PackageInventory(
    val packages: Map<String, PackageInventoryEntry>,
    val profiles: Map<Int, UserProfileInfo>,
    // Running profiles whose package scan genuinely failed — a real problem that
    // blocks loading (an incomplete list must not be saved and silently drop
    // targets).
    val failedUserIds: Set<Int>,
    // Stopped/locked profiles (a locked Private Space, a paused work profile)
    // whose packages cannot be enumerated. This is expected, not an error: the
    // rest of the list still loads and the UI shows a soft notice rather than
    // the blocking failure card.
    val skippedLockedUserIds: Set<Int>,
    val userListComplete: Boolean,
) {
    val complete: Boolean get() = userListComplete && failedUserIds.isEmpty() && packages.isNotEmpty()

    fun incompleteMessage(): String {
        if (!userListComplete) return "Android user list was incomplete"
        if (packages.isEmpty()) return "PackageManager returned no installed packages"
        return "package scan failed for Android user(s): ${failedUserIds.sorted().joinToString()}"
    }
}

internal data class ParsedPackageUidLine(
    val packageName: String,
    val apkPath: String?,
    val uids: List<Int>,
)

/**
 * Parse either supported PackageManager row:
 *
 * - `package:com.example uid:10123`
 * - `package:/data/app/.../base.apk=com.example uid:10123`
 */
internal fun parsePackageUidLine(line: String): ParsedPackageUidLine? {
    val trimmed = line.trim()
    if (!trimmed.startsWith("package:")) return null
    val body = trimmed.removePrefix("package:")
    val uidMarker = body.lastIndexOf(" uid:")
    if (uidMarker <= 0) return null
    val packageToken = body.substring(0, uidMarker).trim()
    val uidToken = body.substring(uidMarker + " uid:".length).trim()
    val separator = packageToken.lastIndexOf('=')
    val packageName = packageToken.substring(separator + 1).trim()
    if (packageName.isEmpty()) return null
    val apkPath =
        packageToken
            .takeIf { separator > 0 }
            ?.substring(0, separator)
            ?.trim()
            ?.takeIf(String::isNotEmpty)
    val uids =
        uidToken
            .split(',')
            .mapNotNull { it.trim().toIntOrNull() }
            .distinct()
            .sorted()
    if (uids.isEmpty()) return null
    return ParsedPackageUidLine(packageName, apkPath, uids)
}

internal fun parsePackageInventory(
    packagesRaw: String,
    usersRaw: String,
): PackageInventory {
    val profiles = parseUserProfiles(usersRaw)
    val userListStatuses = parseUserListStatuses(usersRaw)
    val packageStatuses = linkedMapOf<Int, Int>()
    val usersWithPackages = mutableSetOf<Int>()
    val packages = linkedMapOf<String, MutablePackageInventoryEntry>()
    var currentUserId: Int? = null

    packagesRaw.lineSequence().forEach { line ->
        when {
            line.startsWith(PM_USER_BEGIN_PREFIX) -> {
                currentUserId = line.removePrefix(PM_USER_BEGIN_PREFIX).trim().toIntOrNull()
            }

            line.startsWith(PM_USER_END_PREFIX) -> {
                val fields = line.removePrefix(PM_USER_END_PREFIX).split(':', limit = 2)
                val userId = fields.getOrNull(0)?.toIntOrNull()
                val status = fields.getOrNull(1)?.toIntOrNull()
                if (userId != null && status != null) packageStatuses[userId] = status
                currentUserId = null
            }

            else -> {
                val parsed = parsePackageUidLine(line) ?: return@forEach
                val entry = packages.getOrPut(parsed.packageName) { MutablePackageInventoryEntry() }
                if (entry.apkPath == null) entry.apkPath = parsed.apkPath
                val userIds = currentUserId?.let(::listOf) ?: parsed.uids.map { it / PACKAGE_UIDS_PER_USER }
                userIds.forEach { userId ->
                    usersWithPackages += userId
                    entry.uidsByUser.getOrPut(userId) { linkedSetOf() }.addAll(parsed.uids)
                }
            }
        }
    }

    val expectedUserIds = profiles.keys
    val notEnumerated = expectedUserIds.filter { packageStatuses[it] != 0 || it !in usersWithPackages }
    // A stopped/locked profile can never be enumerated by `pm` — treat that as an
    // expected skip, not a failure. Only a *running* profile that failed to scan
    // blocks the list. Unknown running state (profiles[it]?.running == null) is
    // treated as running, so a profile we could not classify still counts.
    val (skippedLockedUserIds, failedUserIds) =
        notEnumerated.partition { profiles[it]?.running == false }
    val userListComplete = userListStatuses.any { it == 0 } && expectedUserIds.isNotEmpty()
    return PackageInventory(
        packages = packages.mapValues { (_, entry) -> entry.freeze() },
        profiles = profiles,
        failedUserIds = failedUserIds.toSortedSet(),
        skippedLockedUserIds = skippedLockedUserIds.toSortedSet(),
        userListComplete = userListComplete,
    )
}

internal fun requireCompletePackageInventory(sections: Map<String, String>): PackageInventory {
    val inventory =
        parsePackageInventory(
            packagesRaw = sections["pm_packages"].orEmpty(),
            usersRaw = sections["pm_users"].orEmpty(),
        )
    if (!inventory.complete) throw RootSnapshotException(inventory.incompleteMessage())
    return inventory
}

/**
 * One root-shell implementation shared by the normal root snapshot and the
 * exported debug snapshot. It enumerates users first, then queries each user
 * explicitly; relying on `--user all` is incomplete on some OEM ROMs.
 */
internal fun buildPerUserPackageInventoryShell(
    sectionBeginPrefix: String,
    sectionEndPrefix: String,
    stderrRedirect: String,
): String =
    """
    PM_USERS_VERBOSE="${'$'}(pm list users -v $stderrRedirect)"
    PM_USERS_VERBOSE_STATUS=${'$'}?
    PM_USERS_PLAIN="${'$'}(pm list users $stderrRedirect)"
    PM_USERS_PLAIN_STATUS=${'$'}?
    echo "${sectionBeginPrefix}pm_users"
    echo "${PM_USERS_STATUS_PREFIX}verbose:${'$'}PM_USERS_VERBOSE_STATUS"
    echo "${PM_USERS_STATUS_PREFIX}plain:${'$'}PM_USERS_PLAIN_STATUS"
    [ -n "${'$'}PM_USERS_VERBOSE" ] && printf '%s\n' "${'$'}PM_USERS_VERBOSE"
    [ -n "${'$'}PM_USERS_PLAIN" ] && printf '%s\n' "${'$'}PM_USERS_PLAIN"
    echo "${sectionEndPrefix}pm_users"
    PM_PLAIN_USER_IDS="${'$'}(printf '%s\n' "${'$'}PM_USERS_PLAIN" | sed -n 's/.*UserInfo{\([0-9][0-9]*\):.*/\1/p')"
    PM_VERBOSE_USER_IDS="${'$'}(printf '%s\n' "${'$'}PM_USERS_VERBOSE" | sed -n 's/^[[:space:]]*[0-9][0-9]*:[[:space:]]*id=\([0-9][0-9]*\),.*/\1/p')"
    PM_USER_IDS="${'$'}(printf '%s\n%s\n' "${'$'}PM_PLAIN_USER_IDS" "${'$'}PM_VERBOSE_USER_IDS" | sed '/^${'$'}/d' | sort -n -u)"
    echo "${sectionBeginPrefix}pm_packages"
    if [ -z "${'$'}PM_USER_IDS" ]; then
      PM_USER_IDS=0
    fi
    for PM_USER_ID in ${'$'}PM_USER_IDS; do
      PM_USER_PACKAGES="${'$'}(pm list packages -U -f --user "${'$'}PM_USER_ID" $stderrRedirect)"
      PM_USER_STATUS=${'$'}?
      echo "$PM_USER_BEGIN_PREFIX${'$'}PM_USER_ID"
      [ -n "${'$'}PM_USER_PACKAGES" ] && printf '%s\n' "${'$'}PM_USER_PACKAGES"
      echo "$PM_USER_END_PREFIX${'$'}PM_USER_ID:${'$'}PM_USER_STATUS"
    done
    echo "${sectionEndPrefix}pm_packages"
    """.trimIndent()

private data class MutablePackageInventoryEntry(
    var apkPath: String? = null,
    val uidsByUser: MutableMap<Int, MutableSet<Int>> = linkedMapOf(),
) {
    fun freeze(): PackageInventoryEntry =
        PackageInventoryEntry(
            apkPath = apkPath,
            uidsByUser = uidsByUser.mapValues { (_, uids) -> uids.sorted() },
        )
}

private fun parseUserListStatuses(raw: String): List<Int> =
    raw
        .lineSequence()
        .mapNotNull { line ->
            if (!line.startsWith(PM_USERS_STATUS_PREFIX)) return@mapNotNull null
            line.substringAfterLast(':').trim().toIntOrNull()
        }.toList()
