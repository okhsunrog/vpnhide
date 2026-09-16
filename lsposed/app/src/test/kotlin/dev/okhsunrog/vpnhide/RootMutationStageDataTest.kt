package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import java.nio.file.Files
import java.security.MessageDigest
import java.util.UUID

class RootMutationStageDataTest {
    @Test
    fun `staging verifies content and leaves existing executable inode unchanged`() {
        val directory = Files.createTempDirectory("vpnhide-stage").toFile()
        try {
            val source = directory.resolve("source ' with spaces")
            source.writeText("executable content")
            val hash = MessageDigest.getInstance("SHA-256").digest(source.readBytes()).joinToString("") { "%02x".format(it) }
            val destination = directory.resolve("root")
            val command = buildRootMutationStageCommand(source.path, destination.path, hash, UUID.randomUUID().toString())
            val runner = RootProcessRunner()
            assertEquals(RootProcessResult.Completed(""), runner.run(listOf("sh", "-c", command)))
            val target = destination.resolve("vhhelper-$hash")
            val original = Files.getAttribute(target.toPath(), "unix:ino")
            assertEquals(RootProcessResult.Completed(""), runner.run(listOf("sh", "-c", command)))
            assertEquals(original, Files.getAttribute(target.toPath(), "unix:ino"))
            assertFalse(destination.resolve("lane").exists())
            target.writeText("corrupt")
            assertEquals(RootProcessResult.Uncertain, runner.run(listOf("sh", "-c", command)))
        } finally {
            directory.deleteRecursively()
        }
    }
}
