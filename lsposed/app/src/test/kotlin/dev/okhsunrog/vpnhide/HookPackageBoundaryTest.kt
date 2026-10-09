package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files

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

    private val sharedFiles =
        setOf(
            "CanonicalConfigData.kt",
            "HookMaskData.kt",
            "GatedLogger.kt",
            "LogTags.kt",
            "LsposedState.kt",
            "AtomicTelemetryData.kt",
            "PortPolicyData.kt",
            "Protocol.kt",
        )

    private fun kotlinFiles(dir: File): List<File> = dir.walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()

    private fun imports(file: File): List<String> =
        file.readLines().filter { it.startsWith("import ") }.map { it.removePrefix("import ").trim() }

    private fun uiOffenders(files: List<File>): List<String> {
        val forbidden = listOf("androidx.", "android.app.", "android.content.Context", "dev.okhsunrog.vpnhide.ui.")
        return files.flatMap { file ->
            imports(file).filter { imp -> forbidden.any(imp::startsWith) }.map { "${file.name}: $it" }
        }
    }

    @Test
    fun `nested hook and app package imports remain covered`() {
        val fixture = Files.createTempDirectory("hook-boundary").toFile()
        try {
            val hook =
                fixture.resolve("hook/nested/Ui.kt").apply {
                    parentFile?.mkdirs()
                    writeText("import androidx.compose.runtime.Composable")
                }
            val app =
                fixture.resolve("app/nested/Xposed.kt").apply {
                    parentFile?.mkdirs()
                    writeText("import de.robv.android.xposed.XposedBridge")
                }
            assertEquals(listOf("Ui.kt: androidx.compose.runtime.Composable"), uiOffenders(kotlinFiles(fixture.resolve("hook"))))
            assertTrue(kotlinFiles(fixture).containsAll(listOf(hook, app)))
            assertTrue(imports(app).any { it.startsWith("de.robv.android.xposed") })
        } finally {
            fixture.deleteRecursively()
        }
    }

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
        val offenders = uiOffenders(kotlinFiles(hookSources) + sharedFiles.map { File(mainSources, it) })
        assertEquals("hook/ must stay free of app-process UI", emptyList<String>(), offenders)
    }

    @Test
    fun `Xposed APIs stay inside the hook package`() {
        // The reverse direction: XposedBridge is only linked in the LSPosed-loaded
        // process, so touching it from app code throws NoClassDefFoundError.
        val offenders =
            kotlinFiles(mainSources).flatMap { file ->
                if (file.toPath().startsWith(hookSources.toPath())) return@flatMap emptyList()
                imports(file).filter { it.startsWith("de.robv.android.xposed") }.map { "${file.relativeTo(mainSources)}: $it" }
            }
        assertEquals("Xposed imports belong in hook/", emptyList<String>(), offenders)
    }

    @Test
    fun `shared dependencies cannot reach app process declarations`() {
        val files = kotlinFiles(mainSources)
        val shared = files.filter { it.name in sharedFiles && it.parentFile == mainSources }
        assertEquals("reviewed shared sources must exist", sharedFiles.size, shared.size)
        val allowed = shared + kotlinFiles(hookSources) + kotlinFiles(File(mainSources, "generated"))
        val allowedNames = allowed.flatMap { declarations(code(it), includeMembers = true) }.toSet() + "BuildConfig"
        val appOnlyNames = (files - allowed.toSet()).flatMap { declarations(code(it)) }.toSet() - allowedNames
        val offenders =
            allowed.filter { it.parentFile?.name != "generated" }.flatMap { file ->
                val forbiddenImports =
                    imports(file).filter { imported ->
                        imported.startsWith("dev.okhsunrog.vpnhide.") &&
                            !imported.startsWith("dev.okhsunrog.vpnhide.hook.") &&
                            !imported.startsWith("dev.okhsunrog.vpnhide.generated.") &&
                            imported.removePrefix("dev.okhsunrog.vpnhide.") !in allowedNames
                    }
                assertEquals("${file.name}: unreviewed shared imports", emptyList<String>(), forbiddenImports)
                val content = code(file)
                Regex("(?<![\\w.])([A-Za-z_][A-Za-z_0-9]*)\\b(\\s*\\()?")
                    .findAll(content)
                    .filter { it.groupValues[1].first().isUpperCase() || it.groupValues[2].isNotEmpty() }
                    .map { it.groupValues[1] }
                    .filter { it in appOnlyNames }
                    .distinct()
                    .map { "${file.name}: $it" }
                    .toList()
            }
        assertEquals("hook/shared declarations must not reference app-only symbols", emptyList<String>(), offenders)
    }

    private fun code(file: File): String =
        file.readText().replace(Regex("/\\*.*?\\*/|//[^\\n]*|\"\"\".*?\"\"\"|\"(?:\\\\.|[^\"\\\\])*\"", RegexOption.DOT_MATCHES_ALL), " ")

    private fun declarations(
        source: String,
        includeMembers: Boolean = false,
    ): List<String> =
        Regex(
            "(?m)^(?:(?:internal|private|public|protected|data|sealed|suspend|inline|abstract|open|override|const)\\s+)*(?:(?:class|interface|object|typealias)\\s+(\\w+)|fun\\s+[^\\n(]*?\\b(\\w+)\\s*\\(|(?:val|var)\\s+[^\\n:=]*?\\b(\\w+)\\s*[:=])",
        ).findAll(if (includeMembers) source.lines().joinToString("\n") { it.trimStart() } else source)
            .map { match -> match.groupValues.drop(1).first { it.isNotEmpty() } }
            .toList()
}
