package dev.okhsunrog.vpnhide.picker

import dev.okhsunrog.vpnhide.next
import java.util.Locale

private const val ANDROID_UIDS_PER_USER = 100_000

/** VPN Hide has a single supported owner: Android's main user (user 0). */
internal fun isMainAppProfile(uid: Int): Boolean = uid >= 0 && uid / ANDROID_UIDS_PER_USER == 0

/**
 * What kind of Android user a secondary profile is. Derived from
 * `pm list users -v`, which is the only place the type is exposed to a
 * shell — the plain `pm list users` output carries the name only.
 */
internal enum class UserProfileKind {
    WORK,
    CLONE,
    PRIVATE,
    SECONDARY,
    UNKNOWN,
}

/**
 * One row of `pm list users`. [name] is null when the OS reports no name
 * for the profile — clone profiles in particular are commonly created
 * nameless, which is what used to leave the Hiding list showing a bare
 * `(10)` next to an app the user could not place.
 */
internal data class UserProfileInfo(
    val id: Int,
    val name: String?,
    val kind: UserProfileKind,
)

// Verbose row, Android 13+:
//   1: id=10, name=null, type=profile.CLONE, flags=PROFILE|... (parentId=0)
// Android 11/12 print the same row without the `type=` field, so that
// group is optional and the kind falls back to the flags.
private val verboseUserLine =
    Regex("""^\s*\d+:\s*id=(\d+),\s*name=(.*?),\s*(?:type=([^,]+),\s*)?flags=(\S*)""")

// `UserInfo{10:Work:1030}` — flags are trailing hex, name is everything
// between the first `:` and the last `:`. The lazy name-group (.*?)
// combined with a greedy flags-group anchored to `}` handles names that
// contain `:` (rare, but Android allows it). AOSP pins this format, so it
// stays the fallback when the verbose form is unavailable or unparsable.
private val legacyUserLine = Regex("""UserInfo\{(\d+):(.*?):[0-9a-fA-F]+\}""")

/** `name=null` is what the verbose printf emits for a nameless profile. */
private fun normalizeName(raw: String): String? = raw.trim().takeIf { it.isNotEmpty() && it != "null" }

internal fun classifyUserProfile(
    type: String?,
    flags: String?,
): UserProfileKind {
    val t = type?.uppercase(Locale.ROOT).orEmpty()
    val f = flags?.uppercase(Locale.ROOT).orEmpty()
    return when {
        t.contains("MANAGED") || f.contains("MANAGED_PROFILE") -> UserProfileKind.WORK
        t.contains("CLONE") -> UserProfileKind.CLONE
        t.contains("PRIVATE") -> UserProfileKind.PRIVATE
        t.startsWith("FULL") -> UserProfileKind.SECONDARY
        t.startsWith("PROFILE") -> UserProfileKind.UNKNOWN
        f.contains("FULL") -> UserProfileKind.SECONDARY
        else -> UserProfileKind.UNKNOWN
    }
}

/**
 * Parse the users section of the package/user scan. The section holds the
 * output of `pm list users -v` followed by plain `pm list users`, so both
 * formats are accepted: the verbose rows carry the profile type, the plain
 * rows are the naming fallback for ROMs where `-v` isn't supported.
 */
internal fun parseUserProfiles(raw: String): Map<Int, UserProfileInfo> {
    val out = LinkedHashMap<Int, UserProfileInfo>()
    raw.lineSequence().forEach { line ->
        val verbose = verboseUserLine.find(line)
        if (verbose != null) {
            val id = verbose.groupValues[1].toIntOrNull() ?: return@forEach
            out[id] =
                UserProfileInfo(
                    id = id,
                    name = normalizeName(verbose.groupValues[2]),
                    kind = classifyUserProfile(verbose.groupValues[3], verbose.groupValues[4]),
                )
            return@forEach
        }
        val legacy = legacyUserLine.find(line) ?: return@forEach
        val id = legacy.groupValues[1].toIntOrNull() ?: return@forEach
        val name = normalizeName(legacy.groupValues[2])
        val existing = out[id]
        out[id] =
            if (existing == null) {
                UserProfileInfo(id = id, name = name, kind = UserProfileKind.UNKNOWN)
            } else {
                // The verbose row already classified this profile; the plain
                // row can still supply a name the verbose row printed as null.
                existing.copy(name = existing.name ?: name)
            }
    }
    return out
}
