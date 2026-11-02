package dev.okhsunrog.vpnhide.hook

import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Parcel
import de.robv.android.xposed.XposedHelpers
import java.security.MessageDigest

/** Private invocation objects; none are registered with netd or routed. */
internal class FrameworkFixtureObjects(
    val spec: FixtureObjects,
) {
    val vpn = XposedHelpers.newInstance(Network::class.java, spec.vpnNetId) as Network
    val ordinary = XposedHelpers.newInstance(Network::class.java, spec.ordinaryNetId) as Network

    private fun sourceCapabilities(
        transport: Int,
        vpn: Boolean,
    ): NetworkCapabilities =
        (XposedHelpers.newInstance(NetworkCapabilities::class.java) as NetworkCapabilities).also {
            XposedHelpers.callMethod(it, "addTransportType", transport)
            XposedHelpers.callMethod(it, if (vpn) "removeCapability" else "addCapability", NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
        }

    private val vpnCapabilities = sourceCapabilities(NetworkCapabilities.TRANSPORT_VPN, true)
    private val ordinaryCapabilities = sourceCapabilities(NetworkCapabilities.TRANSPORT_WIFI, false)

    private fun sourceProperties(
        name: String,
        mtu: Int,
    ): LinkProperties =
        (XposedHelpers.newInstance(LinkProperties::class.java) as LinkProperties).also {
            XposedHelpers.callMethod(it, "setInterfaceName", name)
            XposedHelpers.callMethod(it, "setMtu", mtu)
        }

    private val vpnProperties = sourceProperties(spec.interfaceName, spec.mtu)
    private val ordinaryProperties = sourceProperties(spec.ordinaryInterfaceName, spec.ordinaryMtu)

    // Constructed and hashed on the fixture observer, outside any worker Binder scope.
    val profileHash: String =
        MessageDigest
            .getInstance("SHA-256")
            .digest(
                bytes(vpnCapabilities) + bytes(vpnProperties) + bytes(ordinaryCapabilities) + bytes(ordinaryProperties),
            ).joinToString("") { "%02x".format(it) }

    fun capabilities(network: Network): NetworkCapabilities? =
        when (network) {
            vpn -> NetworkCapabilities(vpnCapabilities)
            ordinary -> NetworkCapabilities(ordinaryCapabilities)
            else -> null
        }

    fun properties(network: Network): LinkProperties? =
        when (network) {
            vpn -> XposedHelpers.newInstance(LinkProperties::class.java, vpnProperties) as LinkProperties
            ordinary -> XposedHelpers.newInstance(LinkProperties::class.java, ordinaryProperties) as LinkProperties
            else -> null
        }

    private fun bytes(value: android.os.Parcelable): ByteArray {
        val parcel = Parcel.obtain()
        return try {
            value.writeToParcel(parcel, 0)
            parcel.marshall()
        } finally {
            parcel.recycle()
        }
    }
}
