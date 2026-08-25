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
    fun `a failed profile is partial, not empty, and keeps the other profile's packages`() {
        // Root can read every profile regardless of lock state, so a failed
        // profile is fail-soft: the picker still shows what it has (here,
        // user 0's package) plus a banner naming the failed profile — it no
        // longer hard-blocks the whole list.
        val users =
            """
            ${PM_USERS_STATUS_PREFIX}plain:0
            UserInfo{0:Owner:c13}
            UserInfo{10:Work:1030}
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
        assertTrue(inventory.partial)
        assertFalse(inventory.isEmpty)
        assertEquals(setOf(10), inventory.failedUserIds)
        assertTrue(inventory.incompleteMessage().contains("10"))
        assertTrue(inventory.packages.containsKey("com.example"))
    }

    @Test
    fun `a profile that scans successfully but empty is not a failure`() {
        // Motorola vendor profile: user 10 is running, pm exits 0, but returns
        // zero packages. A successful empty scan must not block the whole list.
        val users =
            """
            ${PM_USERS_STATUS_PREFIX}plain:0
            UserInfo{0:Owner:c13} running
            UserInfo{10:Vendor:1001010} running
            """.trimIndent()
        val packages =
            """
            $PM_USER_BEGIN_PREFIX${0}
            package:/system/framework/framework-res.apk=android uid:1000
            $PM_USER_END_PREFIX${0}:0
            $PM_USER_BEGIN_PREFIX${10}
            $PM_USER_END_PREFIX${10}:0
            """.trimIndent()

        val inventory = parsePackageInventory(packages, users)

        assertTrue(inventory.complete)
        assertTrue(inventory.failedUserIds.isEmpty())
    }

    @Test
    fun `a truncated scan with no end marker is still a failure`() {
        // User 10 is listed but its per-user block never closed (no END marker),
        // so its status is unknown — treat that as a failure, not a success.
        val users =
            """
            ${PM_USERS_STATUS_PREFIX}plain:0
            UserInfo{0:Owner:c13} running
            UserInfo{10:Work:1030} running
            """.trimIndent()
        val packages =
            """
            $PM_USER_BEGIN_PREFIX${0}
            package:/system/framework/framework-res.apk=android uid:1000
            $PM_USER_END_PREFIX${0}:0
            $PM_USER_BEGIN_PREFIX${10}
            """.trimIndent()

        assertEquals(setOf(10), parsePackageInventory(packages, users).failedUserIds)
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
                    stderrToStdout = false,
                )
            }
            """.trimIndent()
        val process = ProcessBuilder("sh", "-c", shell).start()
        val stdout = process.inputStream.bufferedReader().readText()
        val stderr = process.errorStream.bufferedReader().readText()

        assertEquals("shell stderr: $stderr", 0, process.waitFor())
        val sections = parseRootShellSnapshot(stdout, recordMetric = { _, _ -> })
        val inventory = requireNonEmptyPackageInventory(sections)
        assertTrue(inventory.complete)
        assertEquals(listOf(10), inventory.packages.getValue("com.work").userIds)
        assertFalse(shell.contains("--user all"))
    }

    @Test
    fun `requireNonEmptyPackageInventory throws only when globally empty`() {
        val usersWithFailedProfile =
            """
            ${PM_USERS_STATUS_PREFIX}plain:0
            UserInfo{0:Owner:c13}
            UserInfo{10:Work:1030}
            """.trimIndent()
        val packagesWithFailedProfile =
            """
            $PM_USER_BEGIN_PREFIX${0}
            package:/data/app/example/base.apk=com.example uid:10123
            $PM_USER_END_PREFIX${0}:0
            $PM_USER_BEGIN_PREFIX${10}
            $PM_USER_END_PREFIX${10}:7
            """.trimIndent()

        // Partial (one failed profile, but user 0 still has a package): no throw.
        val partialInventory =
            requireNonEmptyPackageInventory(
                mapOf("pm_packages" to packagesWithFailedProfile, "pm_users" to usersWithFailedProfile),
            )
        assertTrue(partialInventory.partial)

        // Globally empty (nothing parsed from either section): throws.
        var thrown: RootSnapshotException? = null
        try {
            requireNonEmptyPackageInventory(emptyMap())
        } catch (e: RootSnapshotException) {
            thrown = e
        }
        assertTrue(thrown != null)
    }

    @Test
    fun `mergeUser0Backstop adds a user-0 package the root scan missed`() {
        val backstop = BackstopPackage(packageName = "com.example", apkPath = "/data/app/a", uid = 10123)

        val merged =
            mergeUser0Backstop(
                packages = emptyMap(),
                user0Packages = listOf(backstop),
                currentUserId = 0,
            )

        assertEquals(listOf(0), merged.getValue("com.example").userIds)
        assertEquals(listOf(10123), merged.getValue("com.example").uids)
        assertEquals("/data/app/a", merged.getValue("com.example").apkPath)
    }

    @Test
    fun `mergeUser0Backstop does not override a package the root scan already found for that user`() {
        val existing =
            mapOf(
                "com.example" to
                    PackageInventoryEntry(
                        apkPath = "/data/app/root-scanned",
                        uidsByUser = mapOf(0 to listOf(10123)),
                    ),
            )
        val backstop = BackstopPackage(packageName = "com.example", apkPath = "/data/app/backstop", uid = 99999)

        val merged =
            mergeUser0Backstop(
                packages = existing,
                user0Packages = listOf(backstop),
                currentUserId = 0,
            )

        assertEquals("/data/app/root-scanned", merged.getValue("com.example").apkPath)
        assertEquals(listOf(10123), merged.getValue("com.example").uids)
    }

    @Test
    fun `mergeUser0Backstop fills a missing apkPath but keeps the profile the root scan already has`() {
        val existing =
            mapOf(
                "com.example" to
                    PackageInventoryEntry(apkPath = null, uidsByUser = mapOf(10 to listOf(1010123))),
            )
        val backstop = BackstopPackage(packageName = "com.example", apkPath = "/data/app/backstop", uid = 10123)

        val merged =
            mergeUser0Backstop(
                packages = existing,
                user0Packages = listOf(backstop),
                currentUserId = 0,
            )

        val entry = merged.getValue("com.example")
        assertEquals("/data/app/backstop", entry.apkPath)
        assertEquals(setOf(0, 10), entry.uidsByUser.keys)
        assertEquals(listOf(10123), entry.uidsByUser.getValue(0))
    }
}
