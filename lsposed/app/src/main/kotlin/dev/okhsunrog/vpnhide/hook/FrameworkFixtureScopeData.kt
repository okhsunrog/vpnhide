package dev.okhsunrog.vpnhide.hook

import dev.okhsunrog.vpnhide.LegacyOrdinaryInfo
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject

internal enum class FixturePhase { PREPARED, RAW, PROTECTED, REVOKED }

internal enum class FixtureTransaction { NETWORKS, CAPABILITIES, PROPERTIES, LEGACY_TYPE, LEGACY_ENUM, OTHER }

internal data class FixtureWorker(
    val uid: Int,
    val pid: Int,
    val startTicks: Long,
)

internal data class FixtureObjects(
    val vpnNetId: Int,
    val ordinaryNetId: Int,
    val interfaceName: String,
    val mtu: Int,
    val ordinaryInterfaceName: String,
    val ordinaryMtu: Int,
)

internal data class FixtureAuthority(
    val session: Long,
    val epoch: Long,
    val boot: String,
    val nonce: String,
    val hookHash: String,
    val worker: FixtureWorker,
    val expiry: Long,
    val generation: Int,
    val phase: FixturePhase,
    val objects: FixtureObjects,
) {
    fun permits(
        worker: FixtureWorker,
        boot: String,
        hash: String,
        now: Long,
    ): Boolean = this.worker == worker && this.boot == boot && hookHash == hash && now >= 0 && now < expiry && expiry - now <= 25_000
}

private val fixtureHex = Regex("[0-9a-f]{64}")
private val fixtureBoot = Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")

private fun JsonObject.exact(keys: String) {
    require(this.keys == keys.split(' ').toSet())
}

private fun JsonObject.text(key: String): String = (getValue(key) as JsonPrimitive).also { require(it.isString) }.content

private fun JsonObject.number(key: String): Long =
    (getValue(key) as JsonPrimitive).also { require(!it.isString && Regex("[0-9]+").matches(it.content)) }.content.toLong()

private fun JsonObject.positiveInt(key: String): Int = number(key).also { require(it in 1..Int.MAX_VALUE) }.toInt()

/** Only the fixed Core→system_server protocol is parsed here; verdicts remain in Rust. */
internal fun parseFixtureAuthority(bytes: String): FixtureAuthority? =
    runCatching {
        require(bytes.length in 1..4096)
        val root = Json.parseToJsonElement(bytes).jsonObject
        root.exact(
            "version session core_epoch boot_id nonce expected_hook_sha256 worker_uid worker_pid worker_start_ticks expires_boot_ms generation phase fixture",
        )
        require(root.number("version") == 2L)
        val phase = FixturePhase.valueOf(root.text("phase").uppercase())
        require(root.number("generation") == phase.ordinal.toLong())
        val fixture = root.getValue("fixture").jsonObject
        fixture.exact("vpn_net_id ordinary_net_id interface_name mtu ordinary_interface_name ordinary_mtu")
        val objects =
            FixtureObjects(
                fixture.positiveInt("vpn_net_id"),
                fixture.positiveInt("ordinary_net_id"),
                fixture.text("interface_name"),
                fixture.positiveInt("mtu"),
                fixture.text("ordinary_interface_name"),
                fixture.positiveInt("ordinary_mtu"),
            )
        require(objects.vpnNetId > 65535 && objects.ordinaryNetId > 65535 && objects.vpnNetId != objects.ordinaryNetId)
        require(
            Regex("tun[0-9a-f]{12}").matches(objects.interfaceName) &&
                Regex("ipsec[a-zA-Z0-9]{0,10}").matches(objects.ordinaryInterfaceName),
        )
        require(objects.mtu <= 65535 && objects.ordinaryMtu <= 65535)
        FixtureAuthority(
            root.number("session"),
            root.number("core_epoch"),
            root.text("boot_id"),
            root.text("nonce"),
            root.text("expected_hook_sha256"),
            FixtureWorker(root.positiveInt("worker_uid"), root.positiveInt("worker_pid"), root.number("worker_start_ticks")),
            root.number("expires_boot_ms"),
            phase.ordinal,
            phase,
            objects,
        ).also {
            require(
                it.session > 0 && it.epoch > 0 && fixtureBoot.matches(it.boot) && fixtureHex.matches(it.nonce) &&
                    fixtureHex.matches(it.hookHash),
            )
            require(it.worker.uid >= 10000 && it.worker.pid > 1 && it.worker.startTicks > 0 && it.expiry > 0)
        }
    }.getOrNull()

internal data class FixtureInvocation(
    val authority: FixtureAuthority,
    val transaction: FixtureTransaction,
)

/** Nested untrusted Binder entries must clear the inherited RAW privilege. */
internal class FixtureInvocationScope {
    private val current = ThreadLocal<FixtureInvocation?>()

    fun get(): FixtureInvocation? = current.get()

    fun enter(next: FixtureInvocation?): FixtureInvocation? = current.get().also { current.set(next) }

    fun restore(previous: FixtureInvocation?) {
        current.set(previous)
    }
}

/** Reflection returns equal Method objects through distinct receiver subclasses. */
internal class FixtureDispatchRegistry {
    private val methods = mutableSetOf<java.lang.reflect.Method>()

    fun claim(method: java.lang.reflect.Method): Boolean = methods.add(method)
}

internal data class FixtureLegacyBaseline(
    val ordinary: LegacyOrdinaryInfo,
    val count: Int,
)

internal data class FixtureEvidenceSnapshot(
    val flags: Set<String>,
    val legacyBaseline: FixtureLegacyBaseline?,
    val legacyTypeCalls: Int,
    val legacyEnumerationCalls: Int,
    val legacyTypeStartedNs: Long,
    val legacyEnumerationStartedNs: Long,
)

/** Repeated invocations publish their own baseline even when a flag already exists. */
internal class FixtureEvidenceLedger {
    private val recorded = mutableSetOf<String>()
    val flags: Set<String> get() = recorded.toSet()
    private var legacyTypeStartedNs: Long = 0
    private var legacyEnumerationStartedNs: Long = 0
    var legacyTypeCalls: Int = 0
        private set
    var legacyEnumerationCalls: Int = 0
        private set
    var legacyBaseline: FixtureLegacyBaseline? = null
        private set

    fun snapshot() =
        FixtureEvidenceSnapshot(
            flags,
            legacyBaseline,
            legacyTypeCalls,
            legacyEnumerationCalls,
            legacyTypeStartedNs,
            legacyEnumerationStartedNs,
        )

    fun begin(
        kind: FixtureTransaction,
        bootNanos: Long,
    ) {
        require(bootNanos > 0)
        when (kind) {
            FixtureTransaction.LEGACY_TYPE -> {
                legacyTypeStartedNs = bootNanos
                legacyTypeCalls = (legacyTypeCalls + 1).coerceAtMost(2)
                recorded.removeAll(setOf("legacy_type_source", "legacy_type_normalized"))
            }

            FixtureTransaction.LEGACY_ENUM -> {
                legacyEnumerationStartedNs = bootNanos
                legacyEnumerationCalls = (legacyEnumerationCalls + 1).coerceAtMost(2)
                recorded.removeAll(setOf("legacy_enumeration_source", "legacy_enumeration_normalized"))
                legacyBaseline = null
            }

            else -> {
                Unit
            }
        }
    }

    fun record(
        flag: String,
        baseline: FixtureLegacyBaseline? = null,
    ): Boolean {
        val added = recorded.add(flag)
        if (baseline != null) legacyBaseline = baseline
        return added || baseline != null
    }
}
