package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PackageInventoryDataTest {
    @Test
    fun `merges explicit per-user package rows by package`() {
        val users =
            """
            ${PM_USERS_STATUS_PREFIX}verbose:0
            ${PM_USERS_STATUS_PREFIX}plain:0
            Users:
            UserInfo{0:Owner:c13} running
            UserInfo{10:Work:1030} running
            """.trimIndent()
        val packages =
            """
            $PM_USER_BEGIN_PREFIX${0}
            package:/data/app/telegram/base.apk=org.telegram.messenger uid:10123
            $PM_USER_END_PREFIX${0}:0
            $PM_USER_BEGIN_PREFIX${10}
            package:/data/app/telegram/base.apk=org.telegram.messenger uid:1010123
            package:/data/app/ozon/base.apk=ru.ozon.app uid:1010456
            $PM_USER_END_PREFIX${10}:0
            """.trimIndent()

        val inventory = parsePackageInventory(packages, users)

        assertTrue(inventory.complete)
        assertEquals(listOf(0, 10), inventory.packages.getValue("org.telegram.messenger").userIds)
        assertEquals(listOf(10123, 1010123), inventory.packages.getValue("org.telegram.messenger").uids)
        assertEquals(listOf(10), inventory.packages.getValue("ru.ozon.app").userIds)
        assertEquals("Work", inventory.profiles[10]?.name)
    }

    @Test
    fun `reports a failed running profile instead of accepting a partial list`() {
        val users =
            """
            ${PM_USERS_STATUS_PREFIX}plain:0
            UserInfo{0:Owner:c13} running
            UserInfo{10:Work:1030} running
            """.trimIndent()
        val packages =
            """
            $PM_USER_BEGIN_PREFIX${0}
            package:/data/app/example/base.apk=com.example uid:10123
            $PM_USER_END_PREFIX${0}:0
            $PM_USER_BEGIN_PREFIX${10}
            Error: user 10 is unavailable
            $PM_USER_END_PREFIX${10}:7
            """.trimIndent()

        val inventory = parsePackageInventory(packages, users)

        assertFalse(inventory.complete)
        assertEquals(setOf(10), inventory.failedUserIds)
        assertTrue(inventory.skippedLockedUserIds.isEmpty())
        assertTrue(inventory.incompleteMessage().contains("10"))
    }

    @Test
    fun `a running profile with empty output is a failure`() {
        val users =
            """
            ${PM_USERS_STATUS_PREFIX}plain:0
            UserInfo{0:Owner:c13} running
            UserInfo{10:Private:1030} running
            """.trimIndent()
        val packages =
            """
            $PM_USER_BEGIN_PREFIX${0}
            package:/system/framework/framework-res.apk=android uid:1000
            $PM_USER_END_PREFIX${0}:0
            $PM_USER_BEGIN_PREFIX${10}
            $PM_USER_END_PREFIX${10}:0
            """.trimIndent()

        assertEquals(setOf(10), parsePackageInventory(packages, users).failedUserIds)
    }

    @Test
    fun `a stopped or locked profile is skipped, not a failure`() {
        // No trailing `running` on user 10 — a locked Private Space / paused
        // work profile whose packages `pm` cannot enumerate. The rest of the
        // list still loads and it is reported as a soft skip.
        val users =
            """
            ${PM_USERS_STATUS_PREFIX}plain:0
            Users:
            UserInfo{0:Owner:c13} running
            UserInfo{10:Private space:1090}
            """.trimIndent()
        val packages =
            """
            $PM_USER_BEGIN_PREFIX${0}
            package:/system/framework/framework-res.apk=android uid:1000
            $PM_USER_END_PREFIX${0}:0
            $PM_USER_BEGIN_PREFIX${10}
            Error: user 10 is not running
            $PM_USER_END_PREFIX${10}:7
            """.trimIndent()

        val inventory = parsePackageInventory(packages, users)

        assertTrue(inventory.complete)
        assertTrue(inventory.failedUserIds.isEmpty())
        assertEquals(setOf(10), inventory.skippedLockedUserIds)
        assertFalse(inventory.profiles.getValue(10).running)
    }

    @Test
    fun `shared package uid parser accepts path rows and unions users`() {
        val parsed =
            parsePackageUidMap(
                """
                $PM_USER_BEGIN_PREFIX${0}
                package:/data/app/example/base.apk=com.example uid:10123
                $PM_USER_END_PREFIX${0}:0
                $PM_USER_BEGIN_PREFIX${10}
                package:/data/app/example/base.apk=com.example uid:1010123
                $PM_USER_END_PREFIX${10}:0
                """.trimIndent(),
            )

        assertEquals(listOf(10123, 1010123), parsed["com.example"])
    }

    @Test
    fun `shell scanner queries every listed user without user all`() {
        val shell =
            """
            pm() {
              if [ "${'$'}1 ${'$'}2 ${'$'}3" = "list users -v" ]; then
                echo '0: id=0, name=Owner, type=full.SYSTEM, flags=FULL'
                echo '1: id=10, name=Work, type=profile.MANAGED, flags=PROFILE'
                return 0
              fi
              if [ "${'$'}1 ${'$'}2" = "list users" ]; then
                echo 'Users:'
                echo '  UserInfo{0:Owner:c13} running'
                echo '  UserInfo{10:Work:1030} running'
                return 0
              fi
              if [ "${'$'}6" = "0" ]; then
                echo 'package:/data/app/example/base.apk=com.example uid:10123'
                return 0
              fi
              echo 'package:/data/app/work/base.apk=com.work uid:1010456'
            }
            ${
                buildPerUserPackageInventoryShell(
                    sectionBeginPrefix = "__VPNHIDE_ROOT_SECTION_BEGIN__:",
                    sectionEndPrefix = "__VPNHIDE_ROOT_SECTION_END__:",
                    stderrRedirect = "2>/dev/null",
                )
            }
            """.trimIndent()
        val process = ProcessBuilder("sh", "-c", shell).start()
        val stdout = process.inputStream.bufferedReader().readText()
        val stderr = process.errorStream.bufferedReader().readText()

        assertEquals("shell stderr: $stderr", 0, process.waitFor())
        val sections = parseRootShellSnapshot(stdout, recordMetric = { _, _ -> })
        val inventory = requireCompletePackageInventory(sections)
        assertTrue(inventory.complete)
        assertEquals(listOf(10), inventory.packages.getValue("com.work").userIds)
        assertFalse(shell.contains("--user all"))
    }
}
