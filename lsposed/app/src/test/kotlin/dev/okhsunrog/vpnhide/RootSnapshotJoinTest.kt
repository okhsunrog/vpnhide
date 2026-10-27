package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class RootSnapshotJoinTest {
    @Test
    fun `seeded and freshly probed sections join the same typed publication`() {
        val stale = CanonicalConfig(apps = mapOf("old.app" to CanonicalApp(native = NativeRole.All)))
        val current = CanonicalConfig(apps = mapOf("new.app" to CanonicalApp(java = true)))
        val sections = mapOf("canonical_config" to canonicalConfigJson(stale), "pm_packages" to "package:new.app uid:10123")
        val publication = CoreSessionUpdate(CoreStateView(2, 7, RootCanonicalRead.Available(current), emptyList()), true, sequence = 4)
        val startup = joinRootSnapshot(sections, publication, 1, 0)
        val fresh = joinRootSnapshot(sections.toMutableMap(), publication, 2, 0)
        assertEquals(current, parseTargetsSnapshot(startup).canonicalConfig)
        assertEquals(parseTargetsSnapshot(startup), parseTargetsSnapshot(fresh))
        assertEquals(4L, startup.core.sequence)
        assertEquals(canonicalConfigJson(stale), startup.sections["canonical_config"])
    }

    @Test
    fun `unavailable publication cannot promote forensic file contents to current config`() {
        val config = CanonicalConfig(debug = true)
        val retained = CoreSessionUpdate(CoreStateView(2, 7, RootCanonicalRead.Available(config), emptyList()), current = false)
        val root = joinRootSnapshot(mapOf("canonical_config" to canonicalConfigJson(config)), retained)
        assertNull(parseTargetsSnapshot(root).canonicalConfig)
        assertEquals(RootCanonicalRead.Unavailable, root.canonical)
    }
}
