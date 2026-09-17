package dev.okhsunrog.vpnhide.spike

import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.lifecycle.lifecycleScope
import dev.okhsunrog.vpnhide.ui.theme.VpnHideTheme
import kotlinx.coroutines.launch
import org.json.JSONObject
import uniffi.vpnhide_core_ffi.CoreClientHandle
import uniffi.vpnhide_core_ffi.CoreFfiException

/**
 * THROWAWAY core-2.0 spike harness. Drives the whole app<->core path — the Rust tarpc UDS
 * client behind UniFFI, its async methods as Kotlin `suspend fun`, and the suspend-long-poll
 * feeding a live state view — then prints each step to logcat (tag [TAG]) and on screen.
 *
 * Launch from adb (optionally overriding the socket):
 *   am start -n dev.okhsunrog.vpnhide.dev/dev.okhsunrog.vpnhide.spike.CoreSpikeActivity \
 *            --es socket /data/local/tmp/vpnhide-dev/core.sock
 */
class CoreSpikeActivity : ComponentActivity() {
    private val log = StringBuilder()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val socket =
            intent.getStringExtra("socket") ?: "/data/local/tmp/vpnhide-dev/core.sock"

        var view by mutableStateOf("connecting to $socket …\n")
        setContent {
            VpnHideTheme {
                Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                    val text = remember { view }
                    Column(
                        Modifier
                            .fillMaxSize()
                            .padding(12.dp)
                            .verticalScroll(rememberScrollState()),
                    ) {
                        Text(
                            text = view,
                            fontFamily = FontFamily.Monospace,
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                }
            }
        }

        fun emit(line: String) {
            Log.i(TAG, line)
            log.append(line).append('\n')
            view = log.toString()
        }

        lifecycleScope.launch {
            runScenario(socket, ::emit)
        }
    }

    /** The scripted round-trip. Any UniFFI error is caught and logged, never fatal. */
    private suspend fun runScenario(
        socket: String,
        emit: (String) -> Unit,
    ) {
        val handle =
            try {
                CoreClientHandle.connect(socket)
            } catch (e: CoreFfiException) {
                emit("connect FAILED: ${e.message}")
                return
            }
        emit("connected.")

        // 1) Handshake — proves the async call crosses JNI and the tarpc call completes.
        try {
            val epoch = handle.hello()
            emit("hello OK: core epoch=$epoch")
        } catch (e: CoreFfiException) {
            emit("hello FAILED: ${e.message}")
            return
        }

        // 2) One-shot current state — proves the JSON boundary decodes on the Kotlin side.
        try {
            val json = handle.currentStateJson()
            emit("currentState: ${summarise(json)}")
        } catch (e: CoreFfiException) {
            emit("currentState FAILED: ${e.message}")
        }

        // 3) The suspend-long-poll: a coroutine parks on awaitNextStateJson and prints each
        //    change as it arrives. No polling interval, no callback — this is the StateFlow feed.
        val mirror =
            lifecycleScope.launch {
                var n = 0
                while (true) {
                    val json =
                        try {
                            handle.awaitNextStateJson()
                        } catch (e: CoreFfiException) {
                            emit("mirror stopped: ${e.message}")
                            break
                        }
                    emit("mirror #${++n}: ${summarise(json)}")
                }
            }

        // 4) Bootstrap the config with a Replace edit, then flip a toggle — each should
        //    surface back through the mirror above, proving the write path end to end.
        submit(handle, op = 1uL, baseRevision = 0uL, edit = replaceBootstrap(), emit)
        submit(handle, op = 2uL, baseRevision = 1uL, edit = toggleAutoHideName(), emit)

        // Let the mirror observe a routing tick or two, then wind the harness down.
        kotlinx.coroutines.delay(3_000)
        mirror.cancel()
        emit("scenario done.")
    }

    private suspend fun submit(
        handle: CoreClientHandle,
        op: ULong,
        baseRevision: ULong,
        edit: String,
        emit: (String) -> Unit,
    ) {
        try {
            val outcome = handle.submitEditJson(op, baseRevision, edit)
            emit("submit op=$op base=$baseRevision -> $outcome")
        } catch (e: CoreFfiException) {
            emit("submit op=$op FAILED: ${e.message}")
        }
    }

    /** Pull a few fields out of a StateSnapshot JSON, proving Kotlin can decode the wire. */
    private fun summarise(json: String): String =
        try {
            val o = JSONObject(json)
            val hasConfig = !o.isNull("config")
            val rev = o.optLong("config_revision", -1)
            val routing = o.optJSONObject("routing")
            val observed = routing?.optBoolean("observed") ?: false
            val vpn = routing?.optJSONArray("vpn_interfaces")
            val vpnList =
                buildString {
                    if (vpn != null) {
                        for (i in 0 until vpn.length()) {
                            if (i > 0) append(",")
                            append(vpn.getString(i))
                        }
                    }
                }
            "config=$hasConfig configRev=$rev observed=$observed vpn=[$vpnList]"
        } catch (e: Exception) {
            "unparsable json (${e.message}): ${json.take(120)}"
        }

    private fun replaceBootstrap(): String =
        """
        {"replace":{"config":{
          "version":2,"debug":false,"debugSwitch":false,"apps":{},
          "settings":{
            "rememberSuperkey":false,"optionalFeatures":[],
            "autoHideVpnServices":true,"autoHideVpnName":false,
            "autoHideExcludedPackages":[],"autoHiddenPackages":[]
          }
        }}}
        """.trimIndent()

    private fun toggleAutoHideName(): String = """{"toggle":{"field":"autoHideName","enabled":true}}"""

    companion object {
        private const val TAG = "VpnHide-CoreSpike"
    }
}
