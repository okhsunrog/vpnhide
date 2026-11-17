package dev.okhsunrog.vpnhide.hook

import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkInfo
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
import dev.okhsunrog.vpnhide.appendLegacyFixture
import dev.okhsunrog.vpnhide.legacyInfoFields
import java.util.Collections
import java.util.IdentityHashMap

/** Source hooks supply dirty objects. Production hooks alone produce protected replies. */
internal class FrameworkFixtureHooks {
    private data class State(
        val authority: FixtureAuthority,
        val objects: FrameworkFixtureObjects,
        val evidence: FixtureEvidenceLedger = FixtureEvidenceLedger(),
    ) {
        val flags get() = evidence.flags
        val legacyOrdinary get() = evidence.legacyBaseline?.ordinary
        val legacyCount get() = evidence.legacyBaseline?.count
    }

    private val channel = FrameworkFixtureChannel()
    private val scope = FixtureInvocationScope()
    private val attached = Collections.newSetFromMap(IdentityHashMap<Class<*>, Boolean>())
    private val dispatches = FixtureDispatchRegistry()
    private val guard = Any()

    @Volatile private var state: State? = null

    @Volatile private var receiver: Any? = null

    @Volatile private var networkDispatchInstalled = false
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
        listOf(
            "getActiveNetwork",
            "getAllNetworks",
            "getNetworkCapabilities",
            "getLinkProperties",
            "getNetworkInfo",
            "getAllNetworkInfo",
        ).forEach { name ->
            XposedBridge.hookAllMethods(instance.javaClass, name, sourceHook(name))
        }
        installNetworkDispatch(instance.javaClass)
    }

    private fun installNetworkDispatch(type: Class<*>) {
        val dispatch = runCatching { FrameworkNetworkDispatchFixture(type) }.getOrNull() ?: return
        for (name in listOf("getVpnUnderlyingNetworks", "getNetworkAgentInfoForNetwork")) {
            XposedBridge.hookAllMethods(type, name, networkInputHook(name))
        }
        XposedBridge.hookAllMethods(type, "isNetworkWithCapabilitiesBlocked", ordinaryPolicyInputHook())
        val handles =
            XposedBridge.hookAllMethods(
                type,
                "handleRegisterNetworkRequests",
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        if (state == null || param.hasThrowable()) return
                        val requests = param.args.singleOrNull() as? Set<*> ?: return
                        for (request in requests.filterNotNull()) {
                            runCatching { dispatchRegistration(dispatch, param.thisObject, request) }.onFailure {
                                HookLog.i("diagnostics registration fixture failed: ${it.cause ?: it}")
                            }
                        }
                    }
                },
            )
        networkDispatchInstalled = handles.isNotEmpty()
    }

    private fun networkInputHook(name: String): XC_MethodHook =
        object : XC_MethodHook() {
            override fun beforeHookedMethod(param: MethodHookParam) {
                val invocation = scope.get() ?: return
                if (param.thisObject !== receiver || invocation.transaction !in networkSourceTransactions) return
                val active = current() ?: return
                val argument = param.args.singleOrNull()
                if (name == "getVpnUnderlyingNetworks" && argument == active.authority.worker.uid) {
                    param.result = arrayOf(active.objects.ordinary)
                }
                if (name == "getNetworkAgentInfoForNetwork" && argument == active.objects.ordinary) {
                    invocation.ordinaryAgent?.let { param.result = it }
                }
            }
        }

    private fun ordinaryPolicyInputHook(): XC_MethodHook =
        object : XC_MethodHook() {
            override fun afterHookedMethod(param: MethodHookParam) {
                val invocation = scope.get() ?: return
                val capabilities = invocation.ordinaryCoverCapabilities ?: return
                if (param.thisObject !== receiver || param.args.firstOrNull() !== capabilities ||
                    param.args.getOrNull(1) != invocation.authority.worker.uid || current() == null
                ) {
                    return
                }
                HookLog.i("diagnostics ordinary cover live blocked=${param.result} transaction=${invocation.transaction}")
                param.result = false
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
                val next = State(authority, objects)
                state = next
                channel.acknowledge(authority, objects, next.evidence.snapshot())
            }.onFailure { state = null }
        }
    }

    private fun transactionHook(type: Class<*>): XC_MethodHook =
        object : XC_MethodHook(PRIORITY_HIGHEST) {
            override fun beforeHookedMethod(param: MethodHookParam) {
                val next = if (state == null) null else trustedInvocation(param.thisObject, transaction(type, param.args[0] as Int))
                param.setObjectExtra(scopeKey, scope.enter(next))
                if (next?.transaction in timedFixtureCalls) {
                    synchronized(guard) {
                        val active = current()
                        if (active == null) {
                            param.throwable = IllegalStateException("fixture invocation unavailable")
                            return
                        }
                        active.evidence.begin(requireNotNull(next).transaction, SystemClock.elapsedRealtimeNanos())
                        // An unpublished invocation must never inherit an old normalized ACK.
                        runCatching { acknowledge(active) }.onFailure {
                            param.throwable =
                                IllegalStateException("fixture invocation unavailable")
                        }
                    }
                }
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
                "getActiveNetwork" to FixtureTransaction.ACTIVE_NETWORK,
                "getNetworkInfo" to FixtureTransaction.LEGACY_TYPE,
                "getAllNetworkInfo" to FixtureTransaction.LEGACY_ENUM,
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
                if (name == "getActiveNetwork") {
                    injectActiveNetwork(param, active)
                    return
                }
                if (name == "getNetworkCapabilities" && param.args.firstOrNull() == active.objects.ordinary) {
                    val capabilities = active.objects.capabilities(active.objects.ordinary)
                    val invocation = scope.get()
                    if (invocation != null && invocation.transaction in networkSourceTransactions) {
                        invocation.ordinaryCoverCapabilities = capabilities
                    }
                    param.result = capabilities
                    return
                }
                if (!active.authority.injectsVpn()) return
                when (name) {
                    "getNetworkInfo" -> {
                        injectLegacyType(param, active)
                    }

                    "getAllNetworkInfo" -> {
                        injectLegacyEnumeration(param, active)
                    }

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

    private fun injectActiveNetwork(
        param: XC_MethodHook.MethodHookParam,
        active: State,
    ) {
        if (scope.get()?.transaction != FixtureTransaction.ACTIVE_NETWORK || param.args.isNotEmpty()) return
        synchronized(guard) {
            if (current() !== active) return
            if (!active.authority.injectsVpn()) {
                param.result = active.objects.ordinary
                return
            }
            param.result = active.objects.vpn
            active.evidence.recordActiveSource(active.objects.ordinary.networkHandle)
            runCatching { acknowledge(active) }
        }
    }

    fun activeNetworkReplaced(network: Network) {
        if (scope.get()?.transaction != FixtureTransaction.ACTIVE_NETWORK) return
        synchronized(guard) {
            val active = current() ?: return
            if (active.authority.phase != FixturePhase.PROTECTED || network != active.objects.vpn) return
            active.evidence.recordActiveReplacement()
            runCatching { acknowledge(active) }
        }
    }

    private fun dispatchRegistration(
        dispatch: FrameworkNetworkDispatchFixture,
        instance: Any,
        request: Any,
    ) {
        if (!networkDispatchInstalled) return
        val active = authorizedRegistration(instance, request) ?: return
        val worker = active.authority.worker
        val registration = dispatch.registration(request, worker) ?: return
        logRegistration(active, registration, "start")
        val objects = active.objects
        val ordinary = dispatch.detachedAgent(instance, objects.ordinary, objects, worker.uid)
        val vpn = if (active.authority.injectsVpn()) dispatch.detachedAgent(instance, objects.vpn, objects, worker.uid) else null
        val invocation = beginDispatch(active, registration, ordinary) ?: return
        val previous = scope.enter(invocation)
        try {
            dispatch.send(instance, request, registration, vpn, ordinary) { current() === active }
            logRegistration(active, registration, "returned")
        } finally {
            scope.restore(previous)
        }
    }

    private fun logRegistration(
        active: State,
        registration: FrameworkNetworkDispatchFixture.Registration,
        stage: String,
    ) {
        if (!HookLog.enabled) return
        val kind =
            when (registration) {
                is FrameworkNetworkDispatchFixture.Registration.Callback -> "callback:${registration.kind}"
                is FrameworkNetworkDispatchFixture.Registration.Broadcast -> "pending:${registration.kind}"
            }
        val authority = active.authority
        HookLog.i(
            "diagnostics dispatch session=${authority.session} phase=${authority.phase} worker=${authority.worker.pid} " +
                "registration=$kind stage=$stage boot_ms=${SystemClock.elapsedRealtime()}",
        )
    }

    private fun beginDispatch(
        active: State,
        registration: FrameworkNetworkDispatchFixture.Registration,
        ordinary: Any,
    ): FixtureInvocation? =
        synchronized(guard) {
            if (state !== active || channel.read() != active.authority) return null
            val handle = active.objects.ordinary.networkHandle
            val evidence = active.evidence
            val injected = active.authority.injectsVpn()
            val invocation =
                when (registration) {
                    is FrameworkNetworkDispatchFixture.Registration.Callback -> {
                        if (injected && !evidence.beginCallback(registration.kind, handle)) return null
                        FixtureInvocation(
                            active.authority,
                            FixtureTransaction.CALLBACK,
                            callback = registration.kind,
                            ordinaryAgent = ordinary,
                        )
                    }

                    is FrameworkNetworkDispatchFixture.Registration.Broadcast -> {
                        if (injected && !evidence.beginPendingIntent(registration.kind, handle)) return null
                        FixtureInvocation(
                            active.authority,
                            FixtureTransaction.PENDING_INTENT,
                            pendingIntent = registration.kind,
                            ordinaryAgent = ordinary,
                        )
                    }
                }
            if (injected) acknowledge(active)
            invocation
        }

    fun suppressLiveDispatch(
        instance: Any,
        request: Any,
        uid: Int,
    ): Boolean {
        if (!networkDispatchInstalled || state?.authority?.worker?.uid != uid) return false
        if (scope.get()?.ordinaryAgent != null) return false
        return authorizedRegistration(instance, request) != null
    }

    private fun authorizedRegistration(
        instance: Any,
        request: Any,
    ): State? {
        val active = state ?: return null
        val worker = active.authority.worker
        if (instance !== receiver || !fixtureRequestOwnedBy(request, worker)) return null
        if (fixtureStartTicks(worker.pid) != worker.startTicks || channel.read() != active.authority) return null
        return active
    }

    fun isFixtureCallbackNetwork(network: Network): Boolean {
        val invocation = scope.get() ?: return false
        return (invocation.callback != null || invocation.pendingIntent != null) && current()?.objects?.vpn == network
    }

    fun pendingIntentHandled(
        network: Network,
        cover: Network?,
    ) {
        val registration = scope.get()?.pendingIntent ?: return
        synchronized(guard) {
            val active = current() ?: return
            if (active.authority.phase != FixturePhase.PROTECTED || network != active.objects.vpn) return
            active.evidence.recordPendingIntent(registration, cover?.networkHandle)
            runCatching { acknowledge(active) }
        }
    }

    fun packageCall(
        query: FixturePackageQuery,
        requestedPackage: String?,
    ): FixturePackageCall? {
        val active = state ?: return null
        val authority = active.authority
        val target = authority.objects.packageVisibility ?: return null
        if (requestedPackage != null && requestedPackage != target) return null
        val worker = authority.worker
        if (Binder.getCallingUid() != worker.uid || Binder.getCallingPid() != worker.pid ||
            fixtureStartTicks(worker.pid) != worker.startTicks || channel.read() != authority
        ) {
            return null
        }
        return FixturePackageCall(target, authority.phase == FixturePhase.PROTECTED) { filtered ->
            val start = if (HookLog.enabled) SystemClock.elapsedRealtime() else null
            synchronized(guard) {
                if (state === active && channel.read() == authority && active.evidence.recordPackageSource(query, filtered)) {
                    runCatching { acknowledge(active) }
                }
            }
            if (start != null) {
                HookLog.i(
                    "diagnostics package ACK session=${authority.session} phase=${authority.phase} query=$query " +
                        "worker=${worker.pid} boot_ms=$start duration_ms=${SystemClock.elapsedRealtime() - start}",
                )
            }
        }
    }

    fun callbackHandled(
        network: Network,
        lost: Boolean,
    ) {
        val registration = scope.get()?.callback ?: return
        synchronized(guard) {
            val active = current() ?: return
            if (active.authority.phase != FixturePhase.PROTECTED || network != active.objects.vpn) return
            active.evidence.recordCallback(registration, lost)
            runCatching { acknowledge(active) }
        }
    }

    @Suppress("DEPRECATION")
    private fun injectLegacyType(
        param: XC_MethodHook.MethodHookParam,
        active: State,
    ) {
        if (scope.get()?.transaction != FixtureTransaction.LEGACY_TYPE || param.args.firstOrNull() != ConnectivityManager.TYPE_VPN) return
        if (param.result != null && param.result !is NetworkInfo) return
        param.result = active.objects.copyInfo(active.objects.vpnInfo)
        record("legacy_type_source", FixtureTransaction.LEGACY_TYPE)
    }

    @Suppress("DEPRECATION")
    private fun injectLegacyEnumeration(
        param: XC_MethodHook.MethodHookParam,
        active: State,
    ) {
        if (scope.get()?.transaction != FixtureTransaction.LEGACY_ENUM) return
        val original = param.result as? Array<*> ?: return
        if (original.javaClass.componentType != NetworkInfo::class.java || original.size > 254 ||
            original.any { it !is NetworkInfo }
        ) {
            return
        }
        val next =
            appendLegacyFixture(
                original.filterIsInstance<NetworkInfo>(),
                active.objects.copyInfo(active.objects.vpnInfo),
                active.objects.copyInfo(active.objects.ordinaryInfo),
                active.authority.objects.interfaceName,
                active.authority.objects.ordinaryInterfaceName,
                ::legacyInfoFields,
            ) ?: return
        synchronized(guard) {
            if (current() !== active) return
            param.result = next.infos.toTypedArray()
            record("legacy_enumeration_source", FixtureTransaction.LEGACY_ENUM, FixtureLegacyBaseline(next.ordinary, next.infos.size))
        }
    }

    /** Instrument the real result hook; a parcel-only cleanup cannot impersonate it. */
    @Suppress("DEPRECATION")
    fun legacyNormalized(
        method: String,
        original: NetworkInfo,
    ) {
        val active = current() ?: return
        if (active.authority.phase != FixturePhase.PROTECTED) return
        val tagged = original.type == ConnectivityManager.TYPE_VPN && original.extraInfo == active.authority.objects.interfaceName
        if (!tagged) return
        when (method) {
            "getNetworkInfo" -> record("legacy_type_normalized", FixtureTransaction.LEGACY_TYPE)
            "getAllNetworkInfo" -> record("legacy_enumeration_normalized", FixtureTransaction.LEGACY_ENUM)
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

                    FixtureTransaction.LEGACY_TYPE,
                    FixtureTransaction.LEGACY_ENUM,
                    FixtureTransaction.ACTIVE_NETWORK,
                    FixtureTransaction.CALLBACK,
                    FixtureTransaction.PENDING_INTENT,
                    FixtureTransaction.OTHER,
                    -> return
                },
                kind,
            )
        }
    }

    private fun acknowledge(active: State) {
        channel.acknowledge(active.authority, active.objects, active.evidence.snapshot())
    }

    private fun record(
        flag: String,
        kind: FixtureTransaction,
        baseline: FixtureLegacyBaseline? = null,
    ) {
        if (scope.get()?.transaction != kind) return
        synchronized(guard) {
            val active = current() ?: return
            if (active.evidence.record(flag, baseline)) {
                runCatching {
                    acknowledge(active)
                }
            }
        }
    }

    private val scopeKey = "vpnhide.framework.fixture.previous.${System.identityHashCode(this)}"
    private val timedFixtureCalls = setOf(FixtureTransaction.LEGACY_TYPE, FixtureTransaction.LEGACY_ENUM, FixtureTransaction.ACTIVE_NETWORK)
    private val networkSourceTransactions =
        setOf(FixtureTransaction.ACTIVE_NETWORK, FixtureTransaction.CALLBACK, FixtureTransaction.PENDING_INTENT)
}

private fun FixtureAuthority.injectsVpn(): Boolean = phase == FixturePhase.RAW || phase == FixturePhase.PROTECTED
