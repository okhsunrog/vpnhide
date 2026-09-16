package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.file.Files

class KernelActivatorSelectionTest {
    @Test
    fun `live kernel owner wins over installed companion priority`() {
        assertSelection(status("0x4"), builtin = true, expected = "builtin")
        assertSelection(status("0x0"), builtin = true, expected = "kmod")
        assertSelection(null, builtin = true, expected = "kmod")
    }

    @Test
    fun `unknown malformed or companionless builtin never invokes another activator`() {
        assertSelection(status("0x4"), builtin = false, expected = null)
        assertSelection(status("0x4"), builtin = true, disabled = true, expected = null)
        assertSelection(status("0xff"), builtin = true, expected = null)
        assertSelection("vpnhide 1 status\nbackend 0x4\n", builtin = true, expected = null)
        assertSelection("garbage", builtin = true, expected = null)
        assertSelection(status("0x4") + "backend 0x0\n", builtin = true, expected = null)
    }

    private fun status(backend: String) = "vpnhide 1 status\nbackend $backend\nkver 0x0\nhooks 0x0\nerror 0x0\n"

    private fun assertSelection(
        status: String?,
        builtin: Boolean,
        expected: String?,
        disabled: Boolean = false,
    ) {
        val root = Files.createTempDirectory("vh-selection-").toFile()
        try {
            val calls = root.resolve("calls")
            val ctl = root.resolve("ctl")
            status?.let(ctl::writeText)
            val modules =
                listOf(
                    KMOD_MODULE_DIR to "kmod",
                    BUILTIN_MODULE_DIR to "builtin",
                    KPM_MODULE_DIR to "kpm",
                    ZYGISK_MODULE_DIR to "zygisk",
                )
            var command = ConfigChannels.nativeActivatorCommand().replace(PROC_CTL, ctl.path)
            for ((original, name) in modules) {
                val dir = root.resolve(name).apply { mkdirs() }
                if (name != "builtin" || builtin) dir.resolve("module.prop").writeText("id=test")
                if (name == "builtin" && disabled) dir.resolve("disable").writeText("")
                dir.resolve("activator").apply {
                    writeText("#!/bin/sh\nprintf '%s\\n' '$name' >> '${calls.path}'\n")
                    setExecutable(true)
                }
                command = command.replace(original, dir.path)
            }
            val process = ProcessBuilder("sh", "-c", command).redirectErrorStream(true).start()
            val output = process.inputStream.bufferedReader().readText()
            val exit = process.waitFor()
            if (expected == null) {
                assertTrue(output, exit != 0)
                assertTrue(output, !calls.exists())
            } else {
                assertEquals(output, 0, exit)
                assertEquals(expected, calls.readText().trim())
            }
        } finally {
            root.deleteRecursively()
        }
    }
}
