package dev.okhsunrog.vpnhide

import android.graphics.drawable.Drawable
import dev.okhsunrog.vpnhide.picker.NativeTargetCapacityWarning
import dev.okhsunrog.vpnhide.picker.TargetEntry
import dev.okhsunrog.vpnhide.picker.TargetListGroup
import dev.okhsunrog.vpnhide.picker.TargetListSection
import dev.okhsunrog.vpnhide.picker.TargetListSortMode
import dev.okhsunrog.vpnhide.picker.firstVisibleTargetLabel
import dev.okhsunrog.vpnhide.picker.parseNativeTargetCapacityWarning
import dev.okhsunrog.vpnhide.picker.targetListIndexLabels
import dev.okhsunrog.vpnhide.picker.targetListSections
import dev.okhsunrog.vpnhide.picker.visibleTargetEntries
import org.junit.Assert.assertEquals
import org.junit.Test

class TargetPickerDataTest {
    @Test
    fun `native target capacity warning parses from activator output`() {
        assertEquals(
            NativeTargetCapacityWarning(total = 70, capacity = 64, dropped = 6),
            parseNativeTargetCapacityWarning(
                "unrelated output\nvpnhide-warning native_target_cap total=70 cap=64 dropped=6\n",
            ),
        )
    }

    @Test
    fun `malformed native target capacity warning is ignored`() {
        assertEquals(
            null,
            parseNativeTargetCapacityWarning(
                "vpnhide-warning native_target_cap total=70 cap=64 dropped=5",
            ),
        )
        assertEquals(null, parseNativeTargetCapacityWarning("ordinary activator output"))
    }

    @Test
    fun `configured first keeps alphabetical order inside both groups`() {
        val visible =
            visibleTargetEntries(
                entries =
                    listOf(
                        target("com.zed", "Zed", selected = true),
                        target("com.alpha", "Alpha"),
                        target("com.beta", "beta", selected = true),
                        target("com.gamma", "Gamma"),
                    ),
                searchQuery = "",
                showSystem = false,
                showRussianOnly = false,
                sortMode = TargetListSortMode.ConfiguredFirst,
            )

        assertEquals(listOf("beta", "Zed", "Alpha", "Gamma"), visible.map { it.label })

        val sections = targetListSections(visible, TargetListSortMode.ConfiguredFirst)
        assertEquals(listOf(TargetListGroup.Configured, TargetListGroup.OtherApps), sections.map { it.group })
        assertEquals(listOf("beta", "Zed"), sections[0].entries.map { it.label })
        assertEquals(listOf("Alpha", "Gamma"), sections[1].entries.map { it.label })
    }

    @Test
    fun `configured first groups by saved state while rows have unsaved edits`() {
        val visible =
            visibleTargetEntries(
                entries =
                    listOf(
                        target("com.configured", "Configured", selected = false, groupSelected = true),
                        target("com.other", "Other", selected = true, groupSelected = false),
                    ),
                searchQuery = "",
                showSystem = false,
                showRussianOnly = false,
                sortMode = TargetListSortMode.ConfiguredFirst,
            )

        val sections = targetListSections(visible, TargetListSortMode.ConfiguredFirst)
        assertEquals(listOf(TargetListGroup.Configured, TargetListGroup.OtherApps), sections.map { it.group })
        assertEquals(listOf("Configured"), sections[0].entries.map { it.label })
        assertEquals(listOf("Other"), sections[1].entries.map { it.label })
    }

    @Test
    fun `alphabetical sort ignores configured state`() {
        val visible =
            visibleTargetEntries(
                entries =
                    listOf(
                        target("com.zed", "Zed", selected = true),
                        target("com.alpha", "Alpha"),
                        target("com.beta", "beta", selected = true),
                    ),
                searchQuery = "",
                showSystem = false,
                showRussianOnly = false,
                sortMode = TargetListSortMode.Alphabetical,
            )

        assertEquals(listOf("Alpha", "beta", "Zed"), visible.map { it.label })
        assertEquals(null, targetListSections(visible, TargetListSortMode.Alphabetical).single().group)
    }

    @Test
    fun `configured apps stay visible when russian-only is on`() {
        val visible =
            visibleTargetEntries(
                entries =
                    listOf(
                        target("com.foreign.unselected", "Foreign unselected"),
                        target("com.foreign.selected", "Foreign selected", selected = true),
                        target("ru.bank", "Russian bank"),
                    ),
                searchQuery = "",
                showSystem = false,
                showRussianOnly = true,
                sortMode = TargetListSortMode.ConfiguredFirst,
            )

        // Russian-only narrows discovery to ru.* apps, but a configured foreign
        // app is never hidden — it stays so the user can still see and edit it.
        assertEquals(listOf("Foreign selected", "Russian bank"), visible.map { it.label })
    }

    @Test
    fun `selected system apps stay visible when system apps are hidden`() {
        val visible =
            visibleTargetEntries(
                entries =
                    listOf(
                        target("android.unselected", "Android unselected", isSystem = true),
                        target("android.selected", "Android selected", isSystem = true, selected = true),
                        target("com.user", "User app"),
                    ),
                searchQuery = "",
                showSystem = false,
                showRussianOnly = false,
                sortMode = TargetListSortMode.ConfiguredFirst,
            )

        assertEquals(listOf("Android selected", "User app"), visible.map { it.label })
    }

    @Test
    fun `scrollbar label skips help and section headers`() {
        val sections =
            listOf(
                TargetListSection(TargetListGroup.Configured, listOf(target("com.alpha", "Alpha", selected = true))),
                TargetListSection(TargetListGroup.OtherApps, listOf(target("com.beta", "Beta"))),
            )
        val labels = targetListIndexLabels(sections, hasHelpItem = true)

        assertEquals(listOf(null, null, "Alpha", null, "Beta"), labels)
        assertEquals("A", firstVisibleTargetLabel(labels, 0))
        assertEquals("A", firstVisibleTargetLabel(labels, 1))
        assertEquals("B", firstVisibleTargetLabel(labels, 3))
        assertEquals("B", firstVisibleTargetLabel(labels, 99))
    }

    private fun target(
        packageName: String,
        label: String,
        selected: Boolean = false,
        isSystem: Boolean = false,
        groupSelected: Boolean = selected,
    ): TestTarget =
        TestTarget(
            packageName = packageName,
            label = label,
            isSystem = isSystem,
            selected = selected,
            groupSelected = groupSelected,
        )

    private data class TestTarget(
        override val packageName: String,
        override val label: String,
        override val isSystem: Boolean,
        val selected: Boolean,
        override val groupSelected: Boolean,
    ) : TargetEntry {
        override val icon: Drawable? = null
        override val userIds: List<Int> = emptyList()
        override val anySelected: Boolean get() = selected
    }
}
