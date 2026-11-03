package dev.okhsunrog.vpnhide.picker

import dev.okhsunrog.vpnhide.RootSnapshotException
import uniffi.vpnhide_core_ffi.coreParsePackageInventory

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

/** Local typed Rust codec; Android profile names/kinds remain presentation data. */
internal fun parsePackageInventory(
    packagesRaw: String,
    usersRaw: String,
): PackageInventory {
    val profiles = parseUserProfiles(usersRaw)
    val parsed = coreParsePackageInventory(packagesRaw, usersRaw, profiles.keys.toList())
    return PackageInventory(
        packages = parsed.packages.mapValues { (_, entry) -> PackageInventoryEntry(entry.apkPath, entry.uidsByUser) },
        profiles = profiles,
        failedUserIds = parsed.failedUserIds.toSet(),
        userListComplete = parsed.userListComplete,
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
