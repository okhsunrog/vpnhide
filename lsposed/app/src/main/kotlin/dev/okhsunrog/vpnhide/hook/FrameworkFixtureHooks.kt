package dev.okhsunrog.vpnhide.hook

import android.net.Network
import android.os.Binder
import android.os.FileObserver
import android.os.Handler
import android.os.HandlerThread
import android.os.Parcel
import android.os.SystemClock
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers
import dev.okhsunrog.vpnhide.BuildConfig
import java.util.Collections
import java.util.IdentityHashMap

/** Source hooks supply dirty objects. Production hooks alone produce protected replies. */
internal class FrameworkFixtureHooks {
    private data class State(
        val authority: FixtureAuthority,
        val objects: FrameworkFixtureObjects,
        val flags: MutableSet<String>,
    )

    private val channel = FrameworkFixtureChannel()
    private val scope = FixtureInvocationScope()
    private val attached = Collections.newSetFromMap(IdentityHashMap<Class<*>, Boolean>())
    private val dispatches = FixtureDispatchRegistry()
    private val guard = Any()

    @Volatile private var state: State? = null

    @Volatile private var receiver: Any? = null
    private var observer: FileObserver? = null
    private var handler: Handler? = null

    @Synchronized fun attach(instance: Any) {
        receiver = instance
        if (!attached.add(instance.javaClass)) return
        if (handler == null) {
            val thread = HandlerThread("vpnhide-framework-fixture").apply { start() }
            handler = Handler(thread.looper)
            observer =
                watchSystemDataDir(FileObserver.DELETE or FileObserver.MOVED_FROM) { path ->
                    if (path == FRAMEWORK_AUTHORITY_FILE) handler?.post(::reload)
                }
            handler?.post(::reload)
        }
        val dispatch =
            generateSequence(instance.javaClass as Class<*>?) { it.superclass }
                .takeWhile { it != Binder::class.java }
                .flatMap { it.declaredMethods.asSequence() }
                .firstOrNull {
                    it.name == "onTransact" &&
                        it.parameterTypes.contentEquals(
                            arrayOf(Int::class.javaPrimitiveType, Parcel::class.java, Parcel::class.java, Int::class.javaPrimitiveType),
                        )
                }
                ?: return
        if (dispatches.claim(dispatch)) {
            XposedBridge.hookMethod(dispatch, transactionHook(instance.javaClass))
        }
        listOf("getAllNetworks", "getNetworkCapabilities", "getLinkProperties").forEach { name ->
            XposedBridge.hookAllMethods(instance.javaClass, name, sourceHook(name))
        }
    }

    private fun reload() {
        synchronized(guard) {
            val authority = channel.read()
            if (authority == null) {
                state = null
                return
            }
            val old = state
            if (old?.authority == authority) return
            runCatching {
                val objects =
                    old?.takeIf { it.authority.objects == authority.objects && it.authority.nonce == authority.nonce }?.objects
                        ?: FrameworkFixtureObjects(authority.objects)
                val next = State(authority, objects, mutableSetOf())
                state = next
                channel.acknowledge(authority, objects, next.flags)
            }.onFailure { state = null }
        }
    }

    private fun transactionHook(type: Class<*>): XC_MethodHook =
        object : XC_MethodHook(PRIORITY_HIGHEST) {
            override fun beforeHookedMethod(param: MethodHookParam) {
                val next = if (state == null) null else trustedInvocation(param.thisObject, transaction(type, param.args[0] as Int))
                param.setObjectExtra(scopeKey, scope.enter(next))
            }

            override fun afterHookedMethod(param: MethodHookParam) {
                scope.restore(param.getObjectExtra(scopeKey) as? FixtureInvocation)
            }
        }

    private fun trustedInvocation(
        instance: Any?,
        kind: FixtureTransaction,
    ): FixtureInvocation? {
        val active = state ?: return null
        if (kind == FixtureTransaction.OTHER || instance !== receiver) return null
        val worker = active.authority.worker
        if (Binder.getCallingUid() != worker.uid || Binder.getCallingPid() != worker.pid) return null
        if (fixtureStartTicks(worker.pid) != worker.startTicks || channel.read() != active.authority) return null
        if (!active.authority.permits(
                worker,
                active.authority.boot,
                BuildConfig.HOOK_SOURCE_SHA256,
                SystemClock.elapsedRealtime(),
            )
        ) {
            return null
        }
        return FixtureInvocation(active.authority, kind)
    }

    private fun transaction(
        type: Class<*>,
        code: Int,
    ): FixtureTransaction {
        val names =
            mapOf(
                "getAllNetworks" to FixtureTransaction.NETWORKS,
                "getNetworkCapabilities" to FixtureTransaction.CAPABILITIES,
                "getLinkProperties" to FixtureTransaction.PROPERTIES,
            )
        for (owner in generateSequence(type) { it.superclass }) {
            for ((name, kind) in names) {
                val actual = runCatching { XposedHelpers.getStaticIntField(owner, "TRANSACTION_$name") }.getOrNull()
                if (actual == code) return kind
            }
        }
        return FixtureTransaction.OTHER
    }

    private fun current(): State? {
        val invocation = scope.get() ?: return null
        val active = state ?: return null
        return active.takeIf { it.authority == invocation.authority && channel.read() == it.authority }
    }

    fun rawFor(uid: Int): Boolean =
        current()?.let {
            it.authority.phase == FixturePhase.RAW && it.authority.worker.uid == uid && scope.get()?.transaction != FixtureTransaction.OTHER
        } == true

    private fun sourceHook(name: String): XC_MethodHook =
        object : XC_MethodHook(PRIORITY_LOWEST) {
            override fun afterHookedMethod(param: MethodHookParam) {
                if (param.hasThrowable() || param.thisObject !== receiver) return
                val active = current() ?: return
                if (active.authority.phase !in setOf(FixturePhase.RAW, FixturePhase.PROTECTED)) return
                when (name) {
                    "getAllNetworks" -> {
                        injectNetworks(param, active)
                    }

                    "getNetworkCapabilities" -> {
                        val network = param.args.firstOrNull() as? Network ?: return
                        val replacement = active.objects.capabilities(network) ?: return
                        param.result = replacement
                        if (network == active.objects.vpn) record("capabilities_source", FixtureTransaction.CAPABILITIES)
                    }

                    "getLinkProperties" -> {
                        val network = param.args.firstOrNull() as? Network ?: return
                        val replacement = active.objects.properties(network) ?: return
                        param.result = replacement
                        if (network == active.objects.vpn) record("properties_source", FixtureTransaction.PROPERTIES)
                    }
                }
            }
        }

    private fun injectNetworks(
        param: XC_MethodHook.MethodHookParam,
        active: State,
    ) {
        val original = param.result as? Array<*> ?: return
        if (original.any { it !is Network }) return
        val networks = original.filterIsInstance<Network>()
        if (active.objects.vpn in networks || active.objects.ordinary in networks) return
        param.result = (networks + listOf(active.objects.vpn, active.objects.ordinary)).toTypedArray()
        record("networks_source", FixtureTransaction.NETWORKS)
    }

    fun filtered(
        network: Network,
        kind: FixtureTransaction,
    ) {
        val active = current() ?: return
        if (network == active.objects.vpn && active.authority.phase == FixturePhase.PROTECTED) {
            record(
                when (kind) {
                    FixtureTransaction.NETWORKS -> "networks_filtered"
                    FixtureTransaction.CAPABILITIES -> "capabilities_filtered"
                    FixtureTransaction.PROPERTIES -> "properties_filtered"
                    FixtureTransaction.OTHER -> return
                },
                kind,
            )
        }
    }

    private fun record(
        flag: String,
        kind: FixtureTransaction,
    ) {
        if (scope.get()?.transaction != kind) return
        synchronized(guard) {
            val active = current() ?: return
            if (active.flags.add(flag)) runCatching { channel.acknowledge(active.authority, active.objects, active.flags) }
        }
    }

    private val scopeKey = "vpnhide.framework.fixture.previous.${System.identityHashCode(this)}"
}
