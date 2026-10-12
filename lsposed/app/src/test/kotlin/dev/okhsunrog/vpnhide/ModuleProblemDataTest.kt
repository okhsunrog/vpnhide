package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.KpmProblemKind
import dev.okhsunrog.vpnhide.help.headingSlug
import dev.okhsunrog.vpnhide.help.parseHelpDestination
import dev.okhsunrog.vpnhide.help.resolveHelpLocale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.Locale
import javax.xml.parsers.DocumentBuilderFactory

class ModuleProblemDataTest {
    @Test
    fun cachedProblemsUseCurrentLanguageWithoutReclassification() {
        val english = resourceStrings("values", Locale.ENGLISH)
        val russian = resourceStrings("values-ru", Locale.forLanguageTag("ru"))
        val chinese = resourceStrings("values-zh-rCN", Locale.SIMPLIFIED_CHINESE)
        problems().forEach { cached ->
            val before = cached.renderText(english)
            assertNotEquals(cached.toString(), before, cached.renderText(russian))
            assertNotEquals(cached.toString(), before, cached.renderText(chinese))
            assertEquals(before, cached.renderText(english))
        }
    }

    @Test
    fun diagnosisAndDownloadSurviveLanguageChanges() {
        val cached = ModuleProblem.Kmod(KmodProblemKind.WrongVariant("old-kmi", "new-kmi", "module.zip"))
        listOf("values", "values-ru", "values-zh-rCN").forEach { directory ->
            val text = cached.renderText(resourceStrings(directory, Locale.ROOT))
            listOf("old-kmi", "new-kmi", "module.zip").forEach { assertTrue(text, it in text) }
            assertEquals(ModuleBrokenReason.WrongVariant, cached.reason)
            assertEquals("module.zip", cached.downloadArtifact)
        }
    }

    @Test
    fun missingRuntimeHelpTargetsExistingLocalizedInstallationHeading() {
        val problem = ModuleProblem.Kpm(KpmProblemKind.NoKernelPatchRuntime)
        listOf("en", "ru", "zh", "zh-CN", "de").forEach { language ->
            val destination = parseHelpDestination(problem.helpArticle(language))
            assertEquals("kpm-install", destination.articleId)
            val guide = File("../../docs/help/${resolveHelpLocale(language)}/kpm-install.md")
            val headings = guide.readLines().filter { it.startsWith("## ") }.map { headingSlug(it.removePrefix("## ")) }
            assertTrue("$language: ${destination.anchor}", destination.anchor in headings)
        }
        problems().filter { it != problem }.forEach {
            assertEquals("module-not-active", it.helpArticle("ru"))
        }
    }

    private fun problems(): List<ModuleProblem> =
        listOf(
            KmodProblemKind.KprobesMissing,
            KmodProblemKind.SignatureEnforced,
            KmodProblemKind.UnsupportedKernel("4.14", "module.zip"),
            KmodProblemKind.WrongVariant("old", "new", "module.zip"),
            KmodProblemKind.UnknownVariant("module.zip"),
            KmodProblemKind.AmbiguousLoadFailed("old", "module.zip"),
            KmodProblemKind.LoadFailed("raw error"),
        ).map { ModuleProblem.Kmod(it) } +
            listOf(
                KpmProblemKind.UnsupportedKernel("4.14"),
                KpmProblemKind.NoKernelPatchRuntime,
                KpmProblemKind.NeedsSuperkey,
                KpmProblemKind.LoadFailed("raw error"),
            ).map { ModuleProblem.Kpm(it) } +
            FlashableModuleKind.entries.flatMap { kind ->
                listOf(ActivatorState.Missing, ActivatorState.NotExecutable).map { state ->
                    ModuleProblem.Integrity(ModuleIntegrityProblem(kind, state, "/data/adb/activator"))
                }
            }

    // Exercise the production renderer with actual translations, without Android Resources.
    private fun resourceStrings(
        directory: String,
        locale: Locale,
    ): ModuleProblemStrings {
        val file = File("src/main/res/$directory/strings.xml")
        val nodes =
            DocumentBuilderFactory
                .newInstance()
                .newDocumentBuilder()
                .parse(file)
                .getElementsByTagName("string")
        val strings =
            (0 until nodes.length).associate { index ->
                val node = nodes.item(index)
                node.attributes.getNamedItem("name").nodeValue to node.textContent
            }
        val names = R.string::class.java.fields.associate { it.getInt(null) to it.name }
        return ModuleProblemStrings { id, args ->
            String.format(locale, strings.getValue(names.getValue(id)), *args)
        }
    }
}
