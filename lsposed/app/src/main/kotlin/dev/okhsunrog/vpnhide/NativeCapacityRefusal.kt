package dev.okhsunrog.vpnhide

import android.content.res.Resources
import org.json.JSONObject

/** One native-selected package and the app UIDs (every profile and clone) it adds. */
internal data class NativeCapacityPackage(
    val packageName: String,
    val uids: List<Int>,
)

/**
 * Core refused a native target set that does not fit the backend (`NativeCapacityReport`
 * on the wire). Nothing was saved or applied; the previous config stays in effect. At
 * least [overflow] UIDs must be deselected. [packages] lists most UIDs first.
 */
internal data class NativeCapacityRefusal(
    val total: Int,
    val capacity: Int,
    val packages: List<NativeCapacityPackage>,
) {
    val overflow: Int get() = (total - capacity).coerceAtLeast(0)
}

internal fun parseNativeCapacityReport(json: JSONObject): NativeCapacityRefusal {
    val packages = json.getJSONArray("packages")
    return NativeCapacityRefusal(
        total = json.getInt("total"),
        capacity = json.getInt("capacity"),
        packages =
            List(packages.length()) { index ->
                val entry = packages.getJSONObject(index)
                val uids = entry.getJSONArray("uids")
                NativeCapacityPackage(entry.getString("package"), List(uids.length()) { uids.getInt(it) })
            },
    )
}

/** The relay's `native_capacity_exceeded` detail, or null when it is not a valid report. */
internal fun parseNativeCapacityRefusal(detail: String): NativeCapacityRefusal? =
    runCatching { parseNativeCapacityReport(JSONObject(detail)) }.getOrNull()

private const val LISTED_PACKAGES = 8

/** "Label (3), Other (1), +5 more" — the packages worth deselecting first. */
internal fun NativeCapacityRefusal.packageList(
    res: Resources,
    label: (String) -> String = { it },
): String {
    val shown =
        packages.take(LISTED_PACKAGES).joinToString(", ") {
            res.getString(R.string.native_capacity_package, label(it.packageName), it.uids.size)
        }
    val hidden = packages.size - LISTED_PACKAGES
    return if (hidden > 0) res.getString(R.string.native_capacity_package_more, shown, hidden) else shown
}

/** The Save refusal: what does not fit and what to deselect. */
internal fun NativeCapacityRefusal.saveRefusedMessage(
    res: Resources,
    label: (String) -> String = { it },
): String = res.getString(R.string.save_native_capacity_refused, total, capacity, overflow, packageList(res, label))
