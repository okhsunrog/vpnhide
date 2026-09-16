package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.CallbackEvent
import dev.okhsunrog.vpnhide.debug.CapabilityFacts
import dev.okhsunrog.vpnhide.debug.InvariantResult
import dev.okhsunrog.vpnhide.debug.LegacyTypeView
import dev.okhsunrog.vpnhide.debug.LinkFacts
import dev.okhsunrog.vpnhide.debug.NET_VIEW_NA
import dev.okhsunrog.vpnhide.debug.NET_VIEW_OK
import dev.okhsunrog.vpnhide.debug.NET_VIEW_VIOLATED
import dev.okhsunrog.vpnhide.debug.NetworkFacts
import dev.okhsunrog.vpnhide.debug.NetworkInfoFacts
import dev.okhsunrog.vpnhide.debug.NetworkViewObservations
import dev.okhsunrog.vpnhide.debug.PendingIntentResult
import dev.okhsunrog.vpnhide.debug.evaluateNetworkView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The consistency invariants a target uid's post-Binder network view must hold.
 * Each case is one of the defects the LSPosed consistency work exists to catch,
 * written down as the exact snapshot shape that exposes it.
 */
class NetworkViewDataTest {
    private val wifiCaps = CapabilityFacts(listOf("WIFI"), listOf("INTERNET", "NOT_VPN", "VALIDATED"), "WifiInfo", -1)
    private val wifiLp = LinkFacts("wlan0", listOf("10.0.0.2/24"), listOf("10.0.0.1"), emptyList(), emptyList(), 1500)
    private val wifiInfo = NetworkInfoFacts(1, "WIFI", 0, "CONNECTED", "CONNECTED", true, null)
    private val wifi = NetworkFacts(100, listOf("all", "active"), wifiCaps, wifiLp, wifiInfo)
    private val vpnOff = NetworkInfoFacts(17, "VPN", 0, "DISCONNECTED", "DISCONNECTED", true, null)
    private val legacy =
        LegacyTypeView(
            activeNetworkInfo = wifiInfo,
            byType = mapOf("VPN" to vpnOff, "WIFI" to wifiInfo),
            allNetworkInfo = listOf(wifiInfo, vpnOff),
            networkForType = mapOf("VPN" to null, "WIFI" to 100),
        )

    private fun evaluate(
        active: Int? = 100,
        all: List<Int>? = listOf(100),
        networks: List<NetworkFacts> = listOf(wifi),
        legacyView: LegacyTypeView = legacy,
        phantoms: List<Int> = emptyList(),
        callbacks: List<CallbackEvent> = emptyList(),
        pendingIntent: PendingIntentResult? = null,
        expectHidden: Boolean = true,
        scanComplete: Boolean = true,
        topologyStable: Boolean = true,
    ): Map<String, InvariantResult> =
        evaluateNetworkView(
            NetworkViewObservations(active, all, networks, legacyView, phantoms, callbacks, pendingIntent, scanComplete, topologyStable),
            expectHidden,
        ).associateBy { it.id }

    @Test
    fun `a clean split-tunnel view holds every applicable invariant`() {
        val results = evaluate(callbacks = listOf(CallbackEvent("default", "capabilities", 100, 5, capabilities = wifiCaps)))
        val violated = results.values.filter { it.status == NET_VIEW_VIOLATED }
        assertEquals(emptyList<InvariantResult>(), violated)
        assertEquals(NET_VIEW_OK, results.getValue("callback_matches_sync_view").status)
        assertEquals(NET_VIEW_NA, results.getValue("pending_intent_only_listed_networks").status)
    }

    @Test
    fun `a callback carrying a transport-less capability set under the wifi handle is a violation`() {
        val stripped = CapabilityFacts(emptyList(), listOf("NOT_VPN", "INTERNET"), null, -1)
        val results = evaluate(callbacks = listOf(CallbackEvent("default", "capabilities", 100, 5, capabilities = stripped)))
        val result = results.getValue("callback_matches_sync_view")
        assertEquals(NET_VIEW_VIOLATED, result.status)
        assertTrue(result.detail, result.detail.contains("transports [] vs sync [WIFI]"))
    }

    @Test
    fun `a callback link properties without the interface the sync view reports is a violation`() {
        val bare = LinkFacts(null, emptyList(), emptyList(), emptyList(), emptyList(), -1)
        val results = evaluate(callbacks = listOf(CallbackEvent("default", "link_properties", 100, 7, linkProperties = bare)))
        assertEquals(NET_VIEW_VIOLATED, results.getValue("callback_matches_sync_view").status)
    }

    @Test
    fun `a listed network whose capabilities have no transport is a violation`() {
        val ghost = NetworkFacts(103, listOf("all"), CapabilityFacts(emptyList(), listOf("NOT_VPN"), null, -1), null, null)
        val results = evaluate(all = listOf(100, 103), networks = listOf(wifi, ghost))
        assertEquals("no transport on [103]", results.getValue("listed_networks_have_transport").detail)
    }

    @Test
    fun `a netId that answers outside allNetworks is a phantom`() {
        assertEquals(NET_VIEW_VIOLATED, evaluate(phantoms = listOf(103)).getValue("no_phantom_networks").status)
        assertEquals(NET_VIEW_OK, evaluate().getValue("no_phantom_networks").status)
    }

    @Test
    fun `a connected network without an interface name is a violation`() {
        val bare = wifi.copy(linkProperties = wifiLp.copy(interfaceName = null))
        assertEquals(NET_VIEW_VIOLATED, evaluate(networks = listOf(bare)).getValue("connected_network_has_interface").status)
    }

    @Test
    fun `network info type must name a transport the handle has`() {
        val disguised = wifi.copy(capabilities = CapabilityFacts(listOf("CELLULAR"), listOf("INTERNET"), null, -1))
        val result = evaluate(networks = listOf(disguised)).getValue("info_type_matches_transport")
        assertEquals(NET_VIEW_VIOLATED, result.status)
        assertTrue(result.detail, result.detail.contains("info=WIFI transports=[CELLULAR]"))
    }

    @Test
    fun `active network info must describe the active handle`() {
        val mobileInfo = NetworkInfoFacts(0, "MOBILE", 13, "CONNECTED", "CONNECTED", true, null)
        val results = evaluate(legacyView = legacy.copy(activeNetworkInfo = mobileInfo))
        assertEquals(NET_VIEW_VIOLATED, results.getValue("active_info_matches_active_network").status)
        assertEquals(NET_VIEW_NA, evaluate(active = null).getValue("active_info_matches_active_network").status)
    }

    @Test
    fun `an active handle missing from allNetworks is a violation`() {
        assertEquals(NET_VIEW_VIOLATED, evaluate(active = 103).getValue("active_in_all_networks").status)
    }

    @Test
    fun `callbacks and pending intents may only name listed networks`() {
        val results =
            evaluate(
                callbacks = listOf(CallbackEvent("listen_any", "available", 103, 3), CallbackEvent("listen_any", "lost", 104, 9)),
                pendingIntent = PendingIntentResult(registered = true, netIds = listOf(100), requestNetIds = listOf(103)),
            )
        assertEquals("listen_any/available:103", results.getValue("callbacks_only_for_listed_networks").detail)
        assertEquals(
            "delivered unlisted [103] (listen [100] request [103])",
            results.getValue("pending_intent_only_listed_networks").detail,
        )
    }

    @Test
    fun `vpn-absence invariants apply only when hiding is expected`() {
        val vpnCaps = CapabilityFacts(listOf("VPN"), listOf("INTERNET"), "VpnTransportInfo", -1)
        val vpn =
            NetworkFacts(103, listOf("all"), vpnCaps, LinkFacts("tun0", emptyList(), emptyList(), emptyList(), emptyList(), 1400), null)
        val leaking = evaluate(all = listOf(100, 103), networks = listOf(wifi, vpn))
        assertEquals("TRANSPORT_VPN on [103]", leaking.getValue("no_vpn_transport").detail)
        val baseline = evaluate(all = listOf(100, 103), networks = listOf(wifi, vpn), expectHidden = false)
        assertTrue(baseline.keys.none { it == "no_vpn_transport" || it == "legacy_vpn_inactive" || it == "vpn_listen_silent" })
    }

    @Test
    fun `a connected legacy VPN type or a VPN handle is a leak`() {
        val vpnOn = vpnOff.copy(state = "CONNECTED", detailedState = "CONNECTED")
        val byState = evaluate(legacyView = legacy.copy(byType = mapOf("VPN" to vpnOn)))
        assertEquals(NET_VIEW_VIOLATED, byState.getValue("legacy_vpn_inactive").status)
        val byHandle = evaluate(legacyView = legacy.copy(networkForType = mapOf("VPN" to 103)))
        assertEquals(NET_VIEW_VIOLATED, byHandle.getValue("legacy_vpn_inactive").status)
        assertEquals(NET_VIEW_OK, evaluate().getValue("legacy_vpn_inactive").status)
    }

    @Test
    fun `any delivery on a TRANSPORT_VPN listen is a leak`() {
        val results = evaluate(callbacks = listOf(CallbackEvent("listen_vpn", "available", 103, 4)))
        assertEquals(NET_VIEW_VIOLATED, results.getValue("vpn_listen_silent").status)
    }

    @Test
    fun `failed enumeration is not an empty successful enumeration`() {
        val result = evaluate(all = null)
        assertEquals(NET_VIEW_NA, result.getValue("active_in_all_networks").status)
        assertEquals(NET_VIEW_NA, result.getValue("no_phantom_networks").status)
        assertTrue(result.values.none { it.status == NET_VIEW_VIOLATED })
    }

    @Test
    fun `VPN observed through active handle survives enumeration failure`() {
        val vpn = wifi.copy(capabilities = wifiCaps.copy(transports = listOf("VPN")))
        assertEquals(NET_VIEW_VIOLATED, evaluate(all = null, networks = listOf(vpn)).getValue("no_vpn_transport").status)
    }

    @Test
    fun `failed scan is not proof that phantom networks are absent`() {
        assertEquals(NET_VIEW_NA, evaluate(scanComplete = false).getValue("no_phantom_networks").status)
        assertEquals(NET_VIEW_VIOLATED, evaluate(phantoms = listOf(103), scanComplete = false).getValue("no_phantom_networks").status)
    }

    @Test
    fun `topology change invalidates comparisons but not direct VPN evidence`() {
        val vpn = wifi.copy(capabilities = wifiCaps.copy(transports = listOf("VPN")))
        val results = evaluate(active = 103, networks = listOf(vpn), topologyStable = false)
        assertEquals(NET_VIEW_NA, results.getValue("active_in_all_networks").status)
        assertEquals(NET_VIEW_VIOLATED, results.getValue("no_vpn_transport").status)
    }
}
