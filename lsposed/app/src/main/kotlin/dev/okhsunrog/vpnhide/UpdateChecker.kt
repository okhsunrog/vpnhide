package dev.okhsunrog.vpnhide

import android.content.Context
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

private const val TAG = "VpnHide-Update"
private const val GITHUB_RELEASES_URL =
    "https://api.github.com/repos/okhsunrog/vpnhide/releases/latest"
private const val PREFS_NAME = "vpnhide_prefs"
private const val KEY_LAST_SEEN_VERSION = "last_seen_version"

data class UpdateInfo(
    val latestVersion: String,
    val downloadUrl: String,
)

data class ChangelogData(
    val history: List<ChangelogEntry>,
)

data class ChangelogEntry(
    val version: String,
    val sections: List<ChangelogSection>,
)

data class ChangelogSection(
    val type: String,
    val items: List<BilingualItem>,
)

data class BilingualItem(
    val en: String,
    val ru: String,
)

internal fun normalizeVersion(version: String): String = version.trim().removePrefix("v")

// `git describe --tags --dirty` produces up to two extra suffixes:
//   -<commits>-g<short-sha>   when HEAD is not on a tag
//   -dirty                    when the working tree has uncommitted changes
// Either or both are stripped. Pre-release tags (-rc1, -beta, -alpha.2)
// are preserved because they don't match this shape.
private val GIT_DESCRIBE_DEV_SUFFIX = Regex("""(?:-\d+-g[0-9a-f]+)?(?:-dirty)?$""")

internal fun baseVersion(version: String): String = normalizeVersion(version).replace(GIT_DESCRIBE_DEV_SUFFIX, "")

// True when a module's version is meaningfully different from the app's,
// i.e. both have real base versions and the bases disagree. Dev APKs on
// top of the same release do not count as mismatch.
internal fun versionsMismatch(
    moduleVersion: String?,
    appVersion: String,
): Boolean {
    if (moduleVersion == null) return false
    return baseVersion(moduleVersion) != baseVersion(appVersion)
}

// True when `remote` is a strictly newer release than `current` — used
// to decide whether to offer an in-app update prompt. Both sides go
// through baseVersion() so a dev APK built on top of 0.6.2 still gets
// prompted when 0.6.3 lands (compareSemver by itself returns null for
// the dev suffix, which the old code silently treated as "no update").
internal fun isNewerVersion(
    remote: String,
    current: String,
): Boolean {
    val cmp = compareSemver(baseVersion(remote), baseVersion(current)) ?: return false
    return cmp > 0
}

internal fun compareSemver(
    left: String,
    right: String,
): Int? {
    fun parse(version: String): List<Int>? =
        normalizeVersion(version)
            .split('.')
            .map { it.toIntOrNull() }
            .takeIf { parts ->
                parts.isNotEmpty() && parts.all { it != null }
            }?.map { it!! }

    val l = parse(left) ?: return null
    val r = parse(right) ?: return null
    val maxSize = maxOf(l.size, r.size)
    for (i in 0 until maxSize) {
        val lv = l.getOrElse(i) { 0 }
        val rv = r.getOrElse(i) { 0 }
        if (lv != rv) return lv.compareTo(rv)
    }
    return 0
}

/**
 * Check GitHub Releases for a newer APK version.
 * Returns [UpdateInfo] if a newer version exists, null otherwise.
 * Silently returns null on any error (network, parse, rate limit).
 */
fun checkForUpdate(currentVersion: String): UpdateInfo? {
    try {
        val conn = URL(GITHUB_RELEASES_URL).openConnection() as HttpURLConnection
        conn.setRequestProperty("User-Agent", "vpnhide-android")
        conn.setRequestProperty("Accept", "application/vnd.github+json")
        conn.connectTimeout = 5_000
        conn.readTimeout = 5_000
        try {
            if (conn.responseCode != 200) {
                VpnHideLog.d(TAG, "GitHub API returned ${conn.responseCode}")
                return null
            }
            val body = conn.inputStream.bufferedReader().readText()
            val release = JSONObject(body)
            val remoteVersion = normalizeVersion(release.getString("tag_name"))
            if (!isNewerVersion(remoteVersion, currentVersion)) {
                VpnHideLog.d(TAG, "No update: remote=$remoteVersion current=$currentVersion")
                return null
            }
            val assets = release.getJSONArray("assets")
            var apkUrl: String? = null
            for (i in 0 until assets.length()) {
                val asset = assets.getJSONObject(i)
                if (asset.getString("name").endsWith(".apk")) {
                    apkUrl = asset.getString("browser_download_url")
                    break
                }
            }
            val downloadUrl = apkUrl ?: release.getString("html_url")
            VpnHideLog.i(TAG, "Update available: $remoteVersion (url=$downloadUrl)")
            return UpdateInfo(latestVersion = remoteVersion, downloadUrl = downloadUrl)
        } finally {
            conn.disconnect()
        }
    } catch (e: Exception) {
        VpnHideLog.d(TAG, "Update check failed: ${e.message}")
        return null
    }
}

/** Parse bundled changelog.json from assets. */
fun loadChangelog(context: Context): ChangelogData? =
    try {
        val json =
            context.assets
                .open("changelog.json")
                .bufferedReader()
                .readText()
        val obj = JSONObject(json)
        val history =
            obj.optJSONArray("history")?.let { arr ->
                (0 until arr.length()).map { parseChangelogEntry(arr.getJSONObject(it)) }
            } ?: emptyList()
        ChangelogData(history = history)
    } catch (e: Exception) {
        VpnHideLog.w(TAG, "Failed to load changelog: ${e.message}")
        null
    }

private fun parseChangelogEntry(obj: JSONObject): ChangelogEntry {
    val sections = obj.getJSONArray("sections")
    val sectionList =
        (0 until sections.length()).map { i ->
            val section = sections.getJSONObject(i)
            val items = section.getJSONArray("items")
            ChangelogSection(
                type = section.getString("type"),
                items =
                    (0 until items.length()).map { j ->
                        val item = items.getJSONObject(j)
                        BilingualItem(
                            en = item.getString("en"),
                            ru = item.getString("ru"),
                        )
                    },
            )
        }
    return ChangelogEntry(
        version = obj.getString("version"),
        sections = sectionList,
    )
}

fun shouldShowChangelog(context: Context): Boolean {
    val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    val lastSeen = prefs.getString(KEY_LAST_SEEN_VERSION, null)
    return lastSeen != BuildConfig.VERSION_NAME
}

fun markChangelogSeen(context: Context) {
    context
        .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        .edit()
        .putString(KEY_LAST_SEEN_VERSION, BuildConfig.VERSION_NAME)
        .apply()
}
