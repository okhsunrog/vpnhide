package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NormalizeVersionTest {
    @Test
    fun `strips leading v prefix`() {
        assertEquals("0.6.2", normalizeVersion("v0.6.2"))
        assertEquals("0.6.2", normalizeVersion("0.6.2"))
    }

    @Test
    fun `trims surrounding whitespace`() {
        assertEquals("0.6.2", normalizeVersion("  0.6.2  "))
        assertEquals("0.6.2", normalizeVersion("\n0.6.2\t"))
    }

    @Test
    fun `does not strip inner v`() {
        assertEquals("0.6.2-rcv1", normalizeVersion("0.6.2-rcv1"))
    }

    @Test
    fun `preserves git-describe dev suffix`() {
        // normalizeVersion is not responsible for stripping the dev suffix;
        // baseVersion is.
        assertEquals("0.6.2-14-g1f2205e", normalizeVersion("0.6.2-14-g1f2205e"))
    }
}

class CompareSemverTest {
    @Test
    fun `equal versions return zero`() {
        assertEquals(0, compareSemver("0.6.2", "0.6.2"))
        assertEquals(0, compareSemver("v0.6.2", "0.6.2"))
    }

    @Test
    fun `newer major beats older`() {
        assertTrue((compareSemver("1.0.0", "0.9.9") ?: 0) > 0)
        assertTrue((compareSemver("0.9.9", "1.0.0") ?: 0) < 0)
    }

    @Test
    fun `newer minor and patch compare correctly`() {
        assertTrue((compareSemver("0.7.0", "0.6.9") ?: 0) > 0)
        assertTrue((compareSemver("0.6.3", "0.6.2") ?: 0) > 0)
        assertTrue((compareSemver("0.6.2", "0.6.3") ?: 0) < 0)
    }

    @Test
    fun `missing trailing components treated as zero`() {
        assertEquals(0, compareSemver("0.6", "0.6.0"))
        assertTrue((compareSemver("0.6.1", "0.6") ?: 0) > 0)
    }

    @Test
    fun `non-numeric input returns null for the caller to handle`() {
        // Contract: compareSemver does not attempt to parse suffixes like
        // -rc1 or git-describe dev suffixes. Callers must normalize first
        // (e.g. via baseVersion) if they want those compared.
        assertNull(compareSemver("abc", "0.6.2"))
        assertNull(compareSemver("", "0.6.2"))
    }
}

class BaseVersionTest {
    @Test
    fun `release version passes through unchanged`() {
        assertEquals("0.6.2", baseVersion("0.6.2"))
        assertEquals("1.0.0", baseVersion("1.0.0"))
    }

    @Test
    fun `v prefix still stripped`() {
        assertEquals("0.6.2", baseVersion("v0.6.2"))
    }

    @Test
    fun `git-describe dev suffix is stripped`() {
        // `git describe --tags` appends -<commits>-g<short-sha> when HEAD
        // isn't on a tag. Strip it so a dev APK on top of release v0.6.2
        // compares equal to module.prop version 0.6.2.
        assertEquals("0.6.2", baseVersion("0.6.2-14-g1f2205e"))
        assertEquals("0.6.2", baseVersion("v0.6.2-1-gabcdef0"))
        assertEquals("1.0.0", baseVersion("1.0.0-42-gdeadbee"))
    }

    @Test
    fun `dirty suffix alone is stripped`() {
        // build-version.sh emits `X.Y.Z-dirty` when HEAD is on a tag but
        // the working tree has uncommitted changes.
        assertEquals("0.6.2", baseVersion("0.6.2-dirty"))
        assertEquals("0.6.2", baseVersion("v0.6.2-dirty"))
    }

    @Test
    fun `dev suffix with dirty marker is stripped`() {
        // Most common dev-build shape: -<commits>-g<sha>-dirty.
        assertEquals("0.6.2", baseVersion("0.6.2-14-g1f2205e-dirty"))
        assertEquals("1.0.0", baseVersion("v1.0.0-3-gdeadbee-dirty"))
    }

    @Test
    fun `rc and other pre-release tags are preserved`() {
        // Only the exact -N-gHASH shape is dev-build pollution. Pre-release
        // markers like -rc1, -beta, -alpha.2 must survive so that real
        // version comparisons between pre-releases stay meaningful.
        assertEquals("0.6.2-rc1", baseVersion("0.6.2-rc1"))
        assertEquals("0.6.2-beta", baseVersion("0.6.2-beta"))
        assertEquals("0.6.2-alpha.2", baseVersion("0.6.2-alpha.2"))
    }

    @Test
    fun `whitespace trimmed`() {
        assertEquals("0.6.2", baseVersion("  0.6.2-14-g1f2205e  "))
    }
}

class VersionsMismatchTest {
    @Test
    fun `identical versions do not mismatch`() {
        assertFalse(versionsMismatch("0.6.2", "0.6.2"))
    }

    @Test
    fun `v-prefix difference does not mismatch`() {
        assertFalse(versionsMismatch("v0.6.2", "0.6.2"))
    }

    @Test
    fun `dev-build app on top of release module does not mismatch`() {
        // The false-warning bug: app is built from 14 commits past tag
        // 0.6.2; module is the released 0.6.2. Base versions are equal,
        // so no warning.
        assertFalse(versionsMismatch("0.6.2", "0.6.2-14-g1f2205e"))
        assertFalse(versionsMismatch("0.6.2-14-g1f2205e", "0.6.2"))
    }

    @Test
    fun `two dev builds on same base do not mismatch`() {
        assertFalse(versionsMismatch("0.6.2-3-gabc1234", "0.6.2-14-g1f2205e"))
    }

    @Test
    fun `dirty dev build on same release does not mismatch`() {
        assertFalse(versionsMismatch("0.6.2", "0.6.2-14-g1f2205e-dirty"))
        assertFalse(versionsMismatch("0.6.2", "0.6.2-dirty"))
    }

    @Test
    fun `different release versions do mismatch`() {
        assertTrue(versionsMismatch("0.6.1", "0.6.2"))
        assertTrue(versionsMismatch("0.5.0", "0.6.2"))
    }

    @Test
    fun `dev build on different base does mismatch`() {
        assertTrue(versionsMismatch("0.6.1", "0.6.2-14-g1f2205e"))
    }

    @Test
    fun `null module version never mismatches`() {
        // Unknown/missing module version is handled upstream; don't emit
        // a false mismatch when we simply don't have a module version.
        assertFalse(versionsMismatch(null, "0.6.2"))
    }
}

class IsNewerVersionTest {
    @Test
    fun `newer release beats older release`() {
        assertTrue(isNewerVersion("0.6.3", "0.6.2"))
        assertTrue(isNewerVersion("v0.6.3", "0.6.2"))
        assertTrue(isNewerVersion("1.0.0", "0.9.9"))
    }

    @Test
    fun `equal releases are not newer`() {
        assertFalse(isNewerVersion("0.6.2", "0.6.2"))
        assertFalse(isNewerVersion("v0.6.2", "0.6.2"))
    }

    @Test
    fun `older remote is not newer`() {
        assertFalse(isNewerVersion("0.6.1", "0.6.2"))
    }

    @Test
    fun `dev build on same release gets prompted for new release`() {
        // The bug: without baseVersion() the comparator parses the dev
        // suffix as garbage → returns null → caller treats as "no
        // update" → dev builds never see release notifications.
        assertTrue(isNewerVersion("0.6.3", "0.6.2-14-g1f2205e"))
        assertTrue(isNewerVersion("0.6.3", "0.6.2-14-g1f2205e-dirty"))
    }

    @Test
    fun `dev build on current release is not offered that same release`() {
        // Local is "0.6.2 + 14 commits" — remote 0.6.2 is not newer.
        assertFalse(isNewerVersion("0.6.2", "0.6.2-14-g1f2205e"))
    }

    @Test
    fun `dev build ahead of remote is not offered a downgrade`() {
        // Local is building toward 0.6.3; remote is still on 0.6.2.
        assertFalse(isNewerVersion("0.6.2", "0.6.3-1-gabcdef0"))
    }
}
