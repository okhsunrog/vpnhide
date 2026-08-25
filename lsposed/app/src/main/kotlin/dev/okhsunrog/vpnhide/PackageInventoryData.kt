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
    val failedUserIds: Set<Int>,
    val userListComplete: Boolean,
) {
    val complete: Boolean get() = userListComplete && failedUserIds.isEmpty() && packages.isNotEmpty()

    /** True when the scan yielded at least one package but some profile
     * didn't come back clean — the picker shows what it has plus a soft
     * banner instead of the hard "unlock a profile" wall. */
    val partial: Boolean get() = packages.isNotEmpty() && (!userListComplete || failedUserIds.isNotEmpty())

    /** The only condition that should ever hard-fail the picker: nothing
     * could be enumerated from any source (root scan or the in-process
     * backstop). */
    val isEmpty: Boolean get() = packages.isEmpty()

    fun incompleteMessage(): String {
        if (!userListComplete) return "Android user list was incomplete"
        if (packages.isEmpty()) return "PackageManager returned no installed packages"
        return "package scan failed for Android user(s): ${failedUserIds.sorted().joinToString()}"
    }

    /** Diagnostic-only message naming the profiles that didn't scan cleanly —
     * never shown verbatim in the UI (the picker uses a localized string
     * resource instead), just useful for logs. No "unlock/start" wording:
     * root can read every profile regardless of lock state. */
    fun partialMessage(profileNames: Map<Int, String>): String {
        if (failedUserIds.isEmpty()) return "Android user list was incomplete"
        val names = failedUserIds.sorted().joinToString { profileNames[it] ?: it.toString() }
        return "package scan didn't complete for profile(s): $names"
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
                    entry.uidsByUser.getOrPut(userId) { linkedSetOf() }.addAll(parsed.uids)
                }
            }
        }
    }

    val expectedUserIds = profiles.keys
    // A profile is failed only when its scan did not succeed: a non-zero
    // `pm list packages` exit, or no END marker at all (a truncated scan →
    // status is null → null != 0). An exit-0 scan that returned no packages is
    // a *success* — the profile is legitimately empty (seen on a Motorola
    // vendor profile: user 10 running, exit 0, zero packages). Treating
    // empty-but-successful as a failure used to block the whole app list with
    // "couldn't read all profiles".
    val failedUserIds = expectedUserIds.filterTo(sortedSetOf()) { packageStatuses[it] != 0 }
    val userListComplete = userListStatuses.any { it == 0 } && expectedUserIds.isNotEmpty()
    return PackageInventory(
        packages = packages.mapValues { (_, entry) -> entry.freeze() },
        profiles = profiles,
        failedUserIds = failedUserIds,
        userListComplete = userListComplete,
    )
}

internal fun requireNonEmptyPackageInventory(sections: Map<String, String>): PackageInventory =
    parsePackageInventory(
        packagesRaw = sections["pm_packages"].orEmpty(),
        usersRaw = sections["pm_users"].orEmpty(),
    ).requireNonEmpty()

/**
 * The one hard-fail gate: root can read every Android profile regardless of
 * lock state, so a failed/incomplete profile is almost always a bug, not a
 * genuinely-unreadable one — see [PackageInventory.partial]. Only a globally
 * empty inventory (nothing from any source) still throws.
 */
internal fun PackageInventory.requireNonEmpty(): PackageInventory {
    if (isEmpty) throw RootSnapshotException(incompleteMessage())
    return this
}

/**
 * A minimal, Android-free view of one row from the in-process
 * `getInstalledApplications(0)` backstop — see [mergeUser0Backstop].
 */
internal data class BackstopPackage(
    val packageName: String,
    val apkPath: String?,
    val uid: Int,
)

/**
 * Union the in-process user-0 backstop into a per-user root scan's package
 * map. Pure and Android-free (no [android.content.Context] / `PackageManager`
 * in the signature) so it's unit-testable without Robolectric — the caller
 * resolves [user0Packages] and [currentUserId] (`Process.myUid() / 100_000`)
 * beforehand.
 *
 * A package the root scan already has for [currentUserId] is left untouched;
 * one it's missing (root scan failed/incomplete, or never ran) gets that
 * user id added, with `apkPath` filled in only if the scan didn't already
 * have one. This is what lets the picker still show *something* even when
 * the whole per-user root scan came back empty.
 */
internal fun mergeUser0Backstop(
    packages: Map<String, PackageInventoryEntry>,
    user0Packages: List<BackstopPackage>,
    currentUserId: Int,
): Map<String, PackageInventoryEntry> {
    val merged = packages.toMutableMap()
    user0Packages.forEach { backstop ->
        val existing = merged[backstop.packageName]
        if (existing != null && currentUserId in existing.uidsByUser) return@forEach
        merged[backstop.packageName] =
            PackageInventoryEntry(
                apkPath = existing?.apkPath ?: backstop.apkPath,
                uidsByUser = existing?.uidsByUser.orEmpty() + (currentUserId to listOf(backstop.uid)),
            )
    }
    return merged
}

/**
 * One root-shell implementation shared by the normal root snapshot and the
 * exported debug snapshot. It enumerates users first, then queries each user
 * explicitly; relying on `--user all` is incomplete on some OEM ROMs.
 */
internal fun buildPerUserPackageInventoryShell(
    sectionBeginPrefix: String,
    sectionEndPrefix: String,
    stderrToStdout: Boolean,
): String =
    shellScriptWith(
        "package_inventory.sh",
        mapOf(
            "VPNHIDE_SECTION_BEGIN" to sectionBeginPrefix,
            "VPNHIDE_SECTION_END" to sectionEndPrefix,
            "VPNHIDE_PM_USERS_STATUS" to PM_USERS_STATUS_PREFIX,
            "VPNHIDE_PM_USER_BEGIN" to PM_USER_BEGIN_PREFIX,
            "VPNHIDE_PM_USER_END" to PM_USER_END_PREFIX,
            "VPNHIDE_PM_STDERR_TO_STDOUT" to if (stderrToStdout) "1" else "0",
        ),
    ) + "\nvpnhide_package_inventory\n"

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
