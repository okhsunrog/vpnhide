package dev.okhsunrog.vpnhide.diagnostics

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreInterfacesSample
import uniffi.vpnhide_core_ffi.CoreNativeMeasurement
import uniffi.vpnhide_core_ffi.CoreNativeWorkerInstruction
import uniffi.vpnhide_core_ffi.CoreNativeWorkerPhase
import uniffi.vpnhide_core_ffi.CoreNumberSample
import java.util.concurrent.Executors

class NativeFixtureWorkerTest {
    private fun observation(value: UInt) =
        CoreNativeMeasurement(
            flags = CoreNumberSample.Value(value),
            mtu = CoreNumberSample.Value(value),
            ifaddrs = CoreInterfacesSample.Value(emptyList()),
            links = CoreInterfacesSample.Value(emptyList()),
            ordinaryFlags = CoreNumberSample.Value(73u),
            ordinaryMtu = CoreNumberSample.Value(65536u),
            binding = null,
            addresses = null,
            procInterfaces = null,
        )

    @Test
    fun registrationAndBothObservationsStayOnTheCollectorThreadAcrossRpcSuspension() {
        Executors.newSingleThreadExecutor().asCoroutineDispatcher().use { collector ->
            runBlocking(collector) {
                val originalThread = Thread.currentThread()
                val calls = mutableListOf<String>()
                var count = 0
                val result =
                    measureNativeFixture(
                        step = { phase ->
                            withContext(Dispatchers.Default) { Unit }
                            assertSame(originalThread, Thread.currentThread())
                            when (phase) {
                                CoreNativeWorkerPhase.Prepared -> {
                                    CoreNativeWorkerInstruction.Register(7uL)
                                }

                                CoreNativeWorkerPhase.Registered -> {
                                    CoreNativeWorkerInstruction.ObserveRaw
                                }

                                is CoreNativeWorkerPhase.RawMeasured -> {
                                    assertEquals(observation(1u), phase.native)
                                    calls += "raw-received"
                                    CoreNativeWorkerInstruction.ObserveProtected
                                }
                            }
                        },
                        register = { token ->
                            assertEquals(7uL, token)
                            assertSame(originalThread, Thread.currentThread())
                            calls += "register"
                        },
                        observe = {
                            assertSame(originalThread, Thread.currentThread())
                            count += 1
                            calls += "observe-$count"
                            observation(count.toUInt())
                        },
                    )
                assertEquals(observation(2u), result)
                assertEquals(listOf("register", "observe-1", "raw-received", "observe-2"), calls)
            }
        }
    }

    @Test
    fun noGrantInstructionCollectsOnlyTheProtectedSample() =
        runTest {
            var count = 0
            val result =
                measureNativeFixture(
                    step = { CoreNativeWorkerInstruction.ObserveProtected },
                    register = { error("no registration permitted") },
                    observe = {
                        count += 1
                        observation(2u)
                    },
                )
            assertEquals(1, count)
            assertEquals(observation(2u), result)
        }

    @Test
    fun lostActivationOrRevokeReplyNeverCollectsOrReturnsAPretendProtectedSample() =
        runTest {
            for (failOnActivation in listOf(true, false)) {
                var observations = 0
                val failure =
                    runCatching {
                        measureNativeFixture(
                            step = { phase ->
                                when (phase) {
                                    CoreNativeWorkerPhase.Prepared -> {
                                        CoreNativeWorkerInstruction.Register(7uL)
                                    }

                                    CoreNativeWorkerPhase.Registered -> {
                                        check(!failOnActivation) { "lost activation" }
                                        CoreNativeWorkerInstruction.ObserveRaw
                                    }

                                    is CoreNativeWorkerPhase.RawMeasured -> {
                                        error("unknown revoke")
                                    }
                                }
                            },
                            register = {},
                            observe = {
                                observations += 1
                                observation(1u)
                            },
                        )
                    }
                assertTrue(failure.isFailure)
                assertEquals(if (failOnActivation) 0 else 1, observations)
            }
        }
}
