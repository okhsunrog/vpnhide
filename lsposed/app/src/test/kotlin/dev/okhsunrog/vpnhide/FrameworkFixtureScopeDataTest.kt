package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.hook.FixtureDispatchRegistry
import dev.okhsunrog.vpnhide.hook.FixtureEvidenceLedger
import dev.okhsunrog.vpnhide.hook.FixtureInvocation
import dev.okhsunrog.vpnhide.hook.FixtureInvocationScope
import dev.okhsunrog.vpnhide.hook.FixtureLegacyBaseline
import dev.okhsunrog.vpnhide.hook.FixturePhase
import dev.okhsunrog.vpnhide.hook.FixtureTransaction
import dev.okhsunrog.vpnhide.hook.parseFixtureAuthority
import org.junit.Assert.*
import org.junit.Test
import java.io.File

class FrameworkFixtureScopeDataTest {
    private val golden = File("../../fixtures/diagnostics/framework-envelope.json").readText()

    @Test fun `new Binder invocation clears only its scenario evidence and duplicate calls stay ambiguous`() {
        val ledger = FixtureEvidenceLedger()
        ledger.begin(FixtureTransaction.LEGACY_TYPE, 10)
        ledger.record("legacy_type_source")
        ledger.record("legacy_type_normalized")
        ledger.begin(FixtureTransaction.LEGACY_ENUM, 20)
        val baseline = FixtureLegacyBaseline(LegacyOrdinaryInfo(1, "a".repeat(64), true), 2)
        ledger.record("legacy_enumeration_source", baseline)
        ledger.record("legacy_enumeration_normalized")
        ledger.begin(FixtureTransaction.LEGACY_ENUM, 20)
        assertEquals(1, ledger.legacyTypeCalls)
        assertEquals(2, ledger.legacyEnumerationCalls)
        assertEquals(setOf("legacy_type_source", "legacy_type_normalized"), ledger.flags)
        assertNull(ledger.legacyBaseline)
        repeat(10) { ledger.begin(FixtureTransaction.LEGACY_ENUM, 20) }
        assertEquals(2, ledger.legacyEnumerationCalls)
    }

    @Test fun `repeat enumeration publishes its latest baseline independently of existing flags`() {
        val ledger =
            FixtureEvidenceLedger()
        val first =
            FixtureLegacyBaseline(
                LegacyOrdinaryInfo(1, "a".repeat(64), true),
                2,
            )
        val second = first.copy(ordinary = first.ordinary.copy(count = 2, sha256 = "b".repeat(64)), count = 3)
        val published = mutableListOf<FixtureLegacyBaseline?>()
        for (baseline in listOf(first, second, second)) {
            if (ledger.record("legacy_enumeration_source", baseline)) published += ledger.legacyBaseline
        }
        assertEquals(listOf(first, second, second), published)
        assertEquals(setOf("legacy_enumeration_source"), ledger.flags)
        assertTrue(ledger.record("legacy_enumeration_normalized"))
        assertFalse(ledger.record("legacy_enumeration_normalized"))
    }

    @Test fun `shared envelope binds exact process code boot and lease`() {
        val env = requireNotNull(parseFixtureAuthority(golden))
        assertEquals(FixturePhase.PREPARED, env.phase)
        assertTrue(env.permits(env.worker, env.boot, env.hookHash, 1))
        assertFalse(env.permits(env.worker.copy(pid = 445), env.boot, env.hookHash, 1))
        assertFalse(env.permits(env.worker.copy(startTicks = 1235), env.boot, env.hookHash, 1))
        assertFalse(env.permits(env.worker, env.boot, "d".repeat(64), 1))
        assertFalse(env.permits(env.worker, env.boot, env.hookHash, 25000))
        assertFalse(env.permits(env.worker, env.boot, env.hookHash, -1))
    }

    @Test fun `malformed numeric phase extra key and fixture bounds deny authorization`() {
        for (bad in listOf(
            golden.replace("10123", "10123.0"),
            golden.replace("10123", "\"10123\""),
            golden.replace("\"prepared\"", "\"raw\""),
            golden.replace("1610612737", "2147483648"),
            golden.replace("\"version\": 2", "\"extra\": true, \"version\": 2"),
        )) {
            assertNull(parseFixtureAuthority(bad))
        }
    }

    @Test fun `nested untrusted entry cannot inherit scope and unwind restores caller`() {
        val scope = FixtureInvocationScope()
        val invocation = FixtureInvocation(requireNotNull(parseFixtureAuthority(golden)), FixtureTransaction.NETWORKS)
        val outer = scope.enter(invocation)
        try {
            val nested = scope.enter(null)
            try {
                assertNull(scope.get())
            } finally {
                scope.restore(nested)
            }
            assertEquals(invocation, scope.get())
        } finally {
            scope.restore(outer)
        }
        assertNull(scope.get())
    }

    open class Stub {
        fun dispatch() = Unit
    }

    class FirstReceiver : Stub()

    class SecondReceiver : Stub()

    @Test fun `inherited dispatch is hooked once even through distinct receiver classes`() {
        val registry = FixtureDispatchRegistry()
        assertTrue(registry.claim(FirstReceiver::class.java.getMethod("dispatch")))
        assertFalse(registry.claim(SecondReceiver::class.java.getMethod("dispatch")))
    }
}
