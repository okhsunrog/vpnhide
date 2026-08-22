package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Guards the one boundary in this module that has a real failure mode: code in
 * `hook/` is loaded by LSPosed **into `system_server`**, not into the app.
 *
 * Nothing in the language enforces it — `internal` is module-wide, so a hook
 * file can reference a Compose screen or an app-process cache and still compile,
 * and the mistake only surfaces as a system_server crash on a user's device. The
 * package split makes the boundary visible; this test makes it stick.
 */
class HookPackageBoundaryTest {
    private val mainSources = File("src/main/kotlin/dev/okhsunrog/vpnhide")
    private val hookSources = File(mainSources, "hook")

    private fun kotlinFiles(dir: File): List<File> = dir.listFiles().orEmpty().filter { it.isFile && it.extension == "kt" }

    private fun imports(file: File): List<String> =
        file.readLines().filter { it.startsWith("import ") }.map { it.removePrefix("import ").trim() }

    @Test
    fun `the hook package exists and is where the Xposed entry lives`() {
        assertTrue("hook sources missing at ${hookSources.path}", hookSources.isDirectory)
        assertEquals(
            "assets/xposed_init must name the entry class by its real package",
            "dev.okhsunrog.vpnhide.hook.HookEntry",
            File("src/main/assets/xposed_init").readText().trim(),
        )
    }

    @Test
    fun `hook code pulls in no UI`() {
        // system_server has no Compose, no Activity, no app resources. An import
        // that drags any of it in is a crash waiting for the next ROM.
        val forbidden = listOf("androidx.compose", "androidx.activity", "dev.okhsunrog.vpnhide.ui.")
        val offenders =
            kotlinFiles(hookSources).flatMap { file ->
                imports(file).filter { imp -> forbidden.any(imp::startsWith) }.map { "${file.name}: $it" }
            }
        assertEquals("hook/ must stay free of app-process UI", emptyList<String>(), offenders)
    }

    @Test
    fun `Xposed APIs stay inside the hook package`() {
        // The reverse direction: XposedBridge is only linked in the LSPosed-loaded
        // process, so touching it from app code throws NoClassDefFoundError.
        val offenders =
            kotlinFiles(mainSources).flatMap { file ->
                imports(file).filter { it.startsWith("de.robv.android.xposed") }.map { "${file.name}: $it" }
            }
        assertEquals("Xposed imports belong in hook/", emptyList<String>(), offenders)
    }
}
