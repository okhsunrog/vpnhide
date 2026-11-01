package dev.okhsunrog.vpnhide.hook

import org.junit.Assert.*
import org.junit.Test

class JavaHookReportTest {
    open class Parent {
        fun inherited(value: Int): Int = value
    }

    class Subject : Parent() {
        fun target(value: String): String = value
    }

    @Test fun `signatures name declaring class and parameter and return types without invoking methods`() {
        val methods = describeHookMethods(Subject::class.java, setOf("target", "inherited"))
        assertEquals(2, methods.size)
        assertTrue(methods.any { it.contains("Parent.inherited(int):int") })
        assertTrue(methods.any { it.contains("Subject.target(java.lang.String):java.lang.String") })
        assertTrue(describeHookMethods(Subject::class.java, setOf("missing")).isEmpty())
    }

    @Test fun `report is bounded sanitizes lines and records truncation`() {
        val report = JavaHookReport()
        repeat(1000) { report.record("entry$it", "a\nb\r".repeat(1000)) }
        val snapshot = report.snapshot()
        assertEquals("true", snapshot["java_report_truncated"])
        assertTrue(snapshot.size <= 98)
        assertTrue(snapshot.values.none { '\n' in it || '\r' in it })
        assertTrue(snapshot.values.sumOf { it.length } < 80 * 1024)
    }

    @Test fun `origins are best effort bounded and never private app paths`() {
        val report = JavaHookReport()
        report.loader("/system/framework/${"x".repeat(1100)}.jar")
        assertTrue(report.snapshot().keys.none { it.startsWith("java_origin_") })
        report.loader(
            "PathClassLoader[DexPathList[[zip file \"/apex/com.android.tethering/javalib/service-connectivity.jar\", \"/data/app/private/base.apk\"]]]",
        )
        assertEquals("/apex/com.android.tethering/javalib/service-connectivity.jar", report.snapshot()["java_origin_0"])
        repeat(100) { report.loader("/vendor/framework/oem$it.jar") }
        assertEquals(16, report.snapshot().keys.count { it.startsWith("java_origin_") })
    }
}
