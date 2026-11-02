package dev.okhsunrog.vpnhide

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

    private fun problems(): List<ModuleProblem> =
        FlashableModuleKind.entries.map { kind ->
            ModuleProblem.Integrity(ModuleIntegrityProblem(kind, "/data/adb/modules/x/payload"))
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
