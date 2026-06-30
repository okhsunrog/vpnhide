package dev.okhsunrog.vpnhide

internal enum class TargetListSortMode {
    ConfiguredFirst,
    Alphabetical,
}

internal enum class TargetListGroup {
    Configured,
    OtherApps,
}

internal data class TargetListSection<T : TargetEntry>(
    val group: TargetListGroup?,
    val entries: List<T>,
)

internal fun <T : TargetEntry> visibleTargetEntries(
    entries: List<T>,
    searchQuery: String,
    showSystem: Boolean,
    showRussianOnly: Boolean,
    sortMode: TargetListSortMode,
): List<T> {
    val query = searchQuery.trim().lowercase()
    return entries
        .filter { app ->
            // The system and Russian filters narrow the discovery set for
            // *new* apps; they never hide an already-configured app (anySelected) —
            // its roles are set, so the user must always be able to see and edit
            // it. Search is explicit intent, so it still applies to everything.
            (showSystem || !app.isSystem || app.anySelected) &&
                (!showRussianOnly || isRussianApp(app.packageName, app.label) || app.anySelected) &&
                (
                    query.isEmpty() ||
                        app.label.lowercase().contains(query) ||
                        app.packageName.lowercase().contains(query)
                )
        }.sortedWith(targetEntryComparator(sortMode))
}

internal fun <T : TargetEntry> targetListSections(
    entries: List<T>,
    sortMode: TargetListSortMode,
): List<TargetListSection<T>> {
    if (sortMode == TargetListSortMode.Alphabetical) {
        return listOf(TargetListSection(group = null, entries = entries))
    }

    return listOf(
        TargetListSection(
            group = TargetListGroup.Configured,
            entries = entries.filter { it.anySelected },
        ),
        TargetListSection(
            group = TargetListGroup.OtherApps,
            entries = entries.filterNot { it.anySelected },
        ),
    ).filter { it.entries.isNotEmpty() }
}

internal fun <T : TargetEntry> targetListIndexLabels(
    sections: List<TargetListSection<T>>,
    hasHelpItem: Boolean,
): List<String?> =
    buildList {
        if (hasHelpItem) add(null)
        sections.forEach { section ->
            if (section.group != null) add(null)
            section.entries.forEach { add(it.label) }
        }
    }

internal fun firstVisibleTargetLabel(
    indexLabels: List<String?>,
    firstVisibleItemIndex: Int,
): String {
    indexLabels
        .drop(firstVisibleItemIndex.coerceAtLeast(0))
        .firstOrNull { it != null }
        ?.let { return it.firstOrNull()?.uppercase() ?: "" }

    return indexLabels
        .take(firstVisibleItemIndex.coerceAtMost(indexLabels.size))
        .lastOrNull { it != null }
        ?.firstOrNull()
        ?.uppercase() ?: ""
}

private fun <T : TargetEntry> targetEntryComparator(sortMode: TargetListSortMode): Comparator<T> =
    when (sortMode) {
        TargetListSortMode.ConfiguredFirst -> {
            compareBy<T> { if (it.anySelected) 0 else 1 }
                .then(labelComparator())
        }

        TargetListSortMode.Alphabetical -> {
            labelComparator()
        }
    }

private fun <T : TargetEntry> labelComparator(): Comparator<T> =
    compareBy<T, String>(String.CASE_INSENSITIVE_ORDER) { it.label }
        .thenBy { it.packageName }
