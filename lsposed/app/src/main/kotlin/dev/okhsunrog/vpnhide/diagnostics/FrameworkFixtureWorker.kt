package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.os.Parcel
import android.os.Parcelable
import dev.okhsunrog.vpnhide.BuildConfig
import uniffi.vpnhide_core_ffi.CoreBooleanSample
import uniffi.vpnhide_core_ffi.CoreDiagnosticWorkerTask
import uniffi.vpnhide_core_ffi.CoreFrameworkObservation
import uniffi.vpnhide_core_ffi.CoreFrameworkPhase
import uniffi.vpnhide_core_ffi.CoreFrameworkTask
import uniffi.vpnhide_core_ffi.CoreHandlesSample
import uniffi.vpnhide_core_ffi.CoreOptionalTextSample
import uniffi.vpnhide_core_ffi.CoreProbeError
import uniffi.vpnhide_core_ffi.CoreSession
import uniffi.vpnhide_core_ffi.CoreTextSample
import java.security.MessageDigest

/** Real public ConnectivityManager calls from the Zygote app worker, never precleaned results. */
internal suspend fun measureFrameworkFixture(
    context: Context,
    core: CoreSession,
    task: CoreDiagnosticWorkerTask,
) {
    val manager = context.getSystemService(ConnectivityManager::class.java)
    var previous: CoreFrameworkObservation? = null
    for (phase in listOf(CoreFrameworkPhase.PREPARED, CoreFrameworkPhase.RAW, CoreFrameworkPhase.PROTECTED, CoreFrameworkPhase.REVOKED)) {
        val next = requireNotNull(core.frameworkWorkerStep(task, BuildConfig.HOOK_SOURCE_SHA256, phase, previous))
        previous = observeFramework(manager, next)
    }
    core.frameworkWorkerStep(task, BuildConfig.HOOK_SOURCE_SHA256, CoreFrameworkPhase.FINISHED, previous)
}

@Suppress("DEPRECATION") // getAllNetworks remains a supported detection surface.
private fun observeFramework(
    manager: ConnectivityManager,
    task: CoreFrameworkTask,
): CoreFrameworkObservation {
    val vpn = Network.fromNetworkHandle(task.vpnHandle.toLong())
    val ordinary = Network.fromNetworkHandle(task.ordinaryHandle.toLong())
    // Each field retains its own failure. Null is a successful platform response, not an exception.
    val networks = runCatching { manager.allNetworks.map { it.networkHandle.toULong() }.also { require(it.size <= 256) } }
    val capabilities = runCatching { manager.getNetworkCapabilities(vpn) }
    val properties = runCatching { manager.getLinkProperties(vpn) }
    val ordinaryCapabilities = runCatching { requireNotNull(manager.getNetworkCapabilities(ordinary)).frameworkHash() }
    val ordinaryProperties = runCatching { requireNotNull(manager.getLinkProperties(ordinary)).frameworkHash() }
    return CoreFrameworkObservation(
        networks.fold({ CoreHandlesSample.Value(it) }, { CoreHandlesSample.Error(CoreProbeError.Unsupported) }),
        capabilities.fold({ CoreBooleanSample.Value(it != null) }, { CoreBooleanSample.Error(CoreProbeError.Unsupported) }),
        capabilities.fold({
            CoreBooleanSample.Value(it?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_VPN) == true)
        }, { CoreBooleanSample.Error(CoreProbeError.Unsupported) }),
        properties.fold({ CoreBooleanSample.Value(it != null) }, { CoreBooleanSample.Error(CoreProbeError.Unsupported) }),
        properties.fold({ CoreOptionalTextSample.Value(it?.interfaceName) }, { CoreOptionalTextSample.Error(CoreProbeError.Unsupported) }),
        ordinaryCapabilities.fold({ CoreTextSample.Value(it) }, { CoreTextSample.Error(CoreProbeError.Unsupported) }),
        ordinaryProperties.fold({ CoreTextSample.Value(it) }, { CoreTextSample.Error(CoreProbeError.Unsupported) }),
    )
}

private fun Parcelable.frameworkHash(): String {
    val parcel = Parcel.obtain()
    return try {
        writeToParcel(parcel, 0)
        MessageDigest.getInstance("SHA-256").digest(parcel.marshall()).joinToString("") { "%02x".format(it) }
    } finally {
        parcel.recycle()
    }
}
