package dev.okhsunrog.vpnhide

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.URLClassLoader

class HookCanonicalReaderIsolationTest {
    @Test
    fun `tolerant hook reader keeps explicit ranges and Java policy without any app native classes`() {
        val sources =
            listOf(
                PortRule::class.java,
                JSONObject::class.java,
                Unit::class.java,
                kotlinx.serialization.KSerializer::class.java,
            ).map { requireNotNull(requireNotNull(it.protectionDomain).codeSource).location }.toTypedArray()
        var nativeClassRequests = 0
        val isolated =
            object : URLClassLoader(sources, null) {
                override fun loadClass(
                    name: String,
                    resolve: Boolean,
                ): Class<*> {
                    if (name.startsWith("uniffi.") || name.startsWith("com.sun.jna.")) {
                        nativeClassRequests++
                        throw ClassNotFoundException("native classes deliberately unavailable: $name")
                    }
                    return super.loadClass(name, resolve)
                }
            }
        isolated.use { loader ->
            val reader = Class.forName("dev.okhsunrog.vpnhide.CanonicalConfigDataKt", true, loader)
            val parsed =
                reader.getMethod("parseCanonicalConfig", String::class.java).invoke(
                    null,
                    """{"apps":{
                      "com.bank":{"java":true,"ports":true,"portPolicy":{"mode":"preset","preset":"common_proxy",
                        "rules":[{"protocol":"udp","start":8080},{"protocol":"tcp","start":443}]}},
                      "com.bad":{"java":true,"ports":true,"portPolicy":{"rules":[{"start":0}]}},
                      "com.hidden":{"hidden":true}
                    }}""",
                )
            assertNotNull(parsed)
            val apps = parsed.javaClass.getMethod("getApps").invoke(parsed) as Map<*, *>
            val bank = requireNotNull(apps["com.bank"])
            assertTrue(bank.javaClass.getMethod("getJava").invoke(bank) as Boolean)
            assertTrue(bank.javaClass.getMethod("getPorts").invoke(bank) as Boolean)
            val policy = bank.javaClass.getMethod("getPortPolicy").invoke(bank)
            val rules = policy.javaClass.getMethod("getRules").invoke(policy) as List<*>
            assertEquals(listOf(8080, 443), rules.map { requireNotNull(it).javaClass.getMethod("getStart").invoke(it) })
            val bad = requireNotNull(apps["com.bad"])
            assertTrue(bad.javaClass.getMethod("getJava").invoke(bad) as Boolean)
            assertFalse(bad.javaClass.getMethod("getPorts").invoke(bad) as Boolean)
            assertEquals(0, nativeClassRequests)
        }
    }
}
