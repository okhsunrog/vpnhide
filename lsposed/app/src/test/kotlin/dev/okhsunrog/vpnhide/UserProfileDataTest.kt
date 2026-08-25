package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.UserProfileInfo
import dev.okhsunrog.vpnhide.picker.UserProfileKind
import dev.okhsunrog.vpnhide.picker.classifyUserProfile
import dev.okhsunrog.vpnhide.picker.isMainAppProfile
import dev.okhsunrog.vpnhide.picker.parseUserProfiles
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class UserProfileDataTest {
    @Test
    fun `only android main user can run the app`() {
        assertEquals(true, isMainAppProfile(10_123))
        assertEquals(false, isMainAppProfile(1_010_123))
        assertEquals(false, isMainAppProfile(1_110_123))
        assertEquals(false, isMainAppProfile(-1))
    }

    @Test
    fun `parses verbose rows with profile types`() {
        val raw =
            """
            3 users:

            0: id=0, name=Owner, type=full.SYSTEM, flags=ADMIN|FULL|INITIALIZED|PRIMARY|SYSTEM (running) (current)
            1: id=10, name=null, type=profile.CLONE, flags=INITIALIZED|PROFILE (parentId=0) (running)
            2: id=11, name=Рабочий профиль, type=profile.MANAGED, flags=INITIALIZED|MANAGED_PROFILE|PROFILE (parentId=0)
            """.trimIndent()

        val profiles = parseUserProfiles(raw)

        assertEquals(setOf(0, 10, 11), profiles.keys)
        assertEquals("Owner", profiles[0]?.name)
        assertEquals(UserProfileKind.SECONDARY, profiles[0]?.kind)
        assertNull(profiles[10]?.name)
        assertEquals(UserProfileKind.CLONE, profiles[10]?.kind)
        assertEquals("Рабочий профиль", profiles[11]?.name)
        assertEquals(UserProfileKind.WORK, profiles[11]?.kind)
    }

    @Test
    fun `parses android 11 verbose rows that carry no type field`() {
        val raw =
            """
            2 users:

            0: id=0, name=Owner, flags=ADMIN|FULL|INITIALIZED|PRIMARY|SYSTEM (running) (current)
            1: id=10, name=Work profile, flags=INITIALIZED|MANAGED_PROFILE|PROFILE (running)
            """.trimIndent()

        val profiles = parseUserProfiles(raw)

        assertEquals("Work profile", profiles[10]?.name)
        assertEquals(UserProfileKind.WORK, profiles[10]?.kind)
        assertEquals(UserProfileKind.SECONDARY, profiles[0]?.kind)
    }

    @Test
    fun `falls back to the plain UserInfo format when verbose is unsupported`() {
        val raw =
            """
            Invalid option: -v
            Users:
            	UserInfo{0:Owner:c13} running
            	UserInfo{10:Second Space:10} running
            """.trimIndent()

        val profiles = parseUserProfiles(raw)

        assertEquals(setOf(0, 10), profiles.keys)
        assertEquals("Second Space", profiles[10]?.name)
        assertEquals(UserProfileKind.UNKNOWN, profiles[10]?.kind)
    }

    @Test
    fun `plain rows supply a name the verbose row printed as null`() {
        val raw =
            """
            1: id=10, name=null, type=profile.CLONE, flags=INITIALIZED|PROFILE (parentId=0)
            Users:
            	UserInfo{10:Клон:1030} running
            """.trimIndent()

        val profiles = parseUserProfiles(raw)

        assertEquals("Клон", profiles[10]?.name)
        // The verbose classification survives the merge.
        assertEquals(UserProfileKind.CLONE, profiles[10]?.kind)
    }

    @Test
    fun `plain rows do not overwrite a name the verbose row already had`() {
        val raw =
            """
            1: id=10, name=Verbose name, type=profile.MANAGED, flags=MANAGED_PROFILE|PROFILE
            	UserInfo{10:Legacy name:1030} running
            """.trimIndent()

        assertEquals("Verbose name", parseUserProfiles(raw)[10]?.name)
    }

    @Test
    fun `nameless profiles stay nameless instead of being dropped`() {
        // The old parser skipped empty names outright, which is what left a
        // bare user ID in the Hiding list for nameless clone profiles.
        val profiles = parseUserProfiles("\tUserInfo{10::1030} running")

        assertEquals(setOf(10), profiles.keys)
        assertNull(profiles[10]?.name)
    }

    @Test
    fun `classifies by type first and by flags when the type is absent`() {
        assertEquals(UserProfileKind.WORK, classifyUserProfile("profile.MANAGED", "PROFILE"))
        assertEquals(UserProfileKind.WORK, classifyUserProfile(null, "INITIALIZED|MANAGED_PROFILE"))
        assertEquals(UserProfileKind.CLONE, classifyUserProfile("profile.CLONE", "PROFILE"))
        assertEquals(UserProfileKind.PRIVATE, classifyUserProfile("profile.PRIVATE", "PROFILE"))
        assertEquals(UserProfileKind.SECONDARY, classifyUserProfile("full.SECONDARY", "FULL"))
        assertEquals(UserProfileKind.SECONDARY, classifyUserProfile(null, "FULL|INITIALIZED"))
        // An unrecognised profile type must not be mislabelled as a full user.
        assertEquals(UserProfileKind.UNKNOWN, classifyUserProfile("profile.SUPERVISING", "PROFILE"))
        assertEquals(UserProfileKind.UNKNOWN, classifyUserProfile(null, null))
    }

    @Test
    fun `ignores unrelated output lines`() {
        assertEquals(emptyMap<Int, UserProfileInfo>(), parseUserProfiles("Users:\npackage:/data/app/x=y uid:1010"))
    }
}
