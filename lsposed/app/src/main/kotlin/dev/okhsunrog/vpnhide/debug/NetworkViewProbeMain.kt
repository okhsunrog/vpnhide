package dev.okhsunrog.vpnhide.debug

import android.content.Context
import android.content.ContextWrapper
import android.net.ConnectivityManager
import android.os.IBinder
import android.os.Looper
import android.system.Os
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlin.system.exitProcess

/**
 * Root `app_process` entry point: take a [NetworkViewSnapshot] as ANY uid, so a
 * target and a non-target app's views can be compared post-Binder without
 * installing anything. Driven by `scripts/network-view-probe.py`:
 *
 *     CLASSPATH=<this APK> app_process /system/bin \
 *         dev.okhsunrog.vpnhide.debug.NetworkViewProbeMain --uid 10332 --package <pkg>
 *
 * The process starts as root (SELinux context of the su shell), drops to the
 * requested uid with setgid/setuid, and then talks to ConnectivityService as
 * that uid with the package's name as the op package — which is what the
 * service's package/uid checks compare. No Application, no Looper of its own;
 * the callback registrations use ConnectivityManager's own thread. Prints one
 * JSON document (the same shape the debug bundle embeds) on stdout.
 *
 * Kept by proguard-rules.pro; nothing else in the app references it.
 */
internal object NetworkViewProbeMain {
    private val json =
        Json {
            prettyPrint = true
            encodeDefaults = true
        }

    @JvmStatic
    @Suppress("DEPRECATION") // Os.setgid/setuid are the only way to drop uid here.
    fun main(args: Array<String>) {
        val opts = parseArgs(args)
        // ActivityThread's handler needs a Looper on this thread; nothing ever
        // loops it — the callbacks arrive on ConnectivityManager's own thread.
        Looper.prepare()
        val systemContext = systemContext()
        val service = connectivityService()
        val context = packageContext(systemContext, opts.packageName)
        Os.setgid(opts.uid)
        Os.setuid(opts.uid)
        val cm =
            ConnectivityManager::class.java
                .getConstructor(Context::class.java, Class.forName("android.net.IConnectivityManager"))
                .newInstance(context, service)
        val snapshot =
            captureNetworkView(
                context,
                cm,
                opts.uid,
                opts.packageName,
                NetworkViewOptions(
                    captureMs = opts.captureMs,
                    expectHidden = opts.expectHidden,
                    includePendingIntent = false,
                    scanBeyond = opts.scanBeyond,
                ),
            )
        println(json.encodeToString(snapshot))
        System.out.flush()
        // ConnectivityThread is a non-daemon HandlerThread; the VM would otherwise
        // outlive main() and the caller would wait for output that never ends.
        exitProcess(0)
    }

    private data class ProbeArgs(
        val uid: Int,
        val packageName: String,
        val captureMs: Long,
        val expectHidden: Boolean,
        val scanBeyond: Int,
    )

    private fun parseArgs(args: Array<String>): ProbeArgs {
        var uid = -1
        var packageName = ""
        var captureMs = 2_000L
        var expectHidden = false
        var scanBeyond = 30
        var i = 0
        while (i < args.size) {
            when (args[i]) {
                "--uid" -> uid = args[++i].toInt()
                "--package" -> packageName = args[++i]
                "--capture-ms" -> captureMs = args[++i].toLong()
                "--scan-beyond" -> scanBeyond = args[++i].toInt()
                "--expect-hidden" -> expectHidden = true
                else -> error("unknown argument ${args[i]}")
            }
            i++
        }
        require(uid >= 0) { "--uid is required" }
        require(packageName.isNotEmpty()) { "--package is required" }
        return ProbeArgs(uid, packageName, captureMs, expectHidden, scanBeyond)
    }

    // ActivityThread.systemMain() is the standard way a command-line Java process
    // gets a usable Context (the shell's own `settings`/`content` tools do this).
    private fun systemContext(): Context {
        val activityThread = Class.forName("android.app.ActivityThread")
        val thread = activityThread.getMethod("systemMain").invoke(null)
        return activityThread.getMethod("getSystemContext").invoke(thread) as Context
    }

    private fun connectivityService(): Any {
        val serviceManager = Class.forName("android.os.ServiceManager")
        val binder = serviceManager.getMethod("getService", String::class.java).invoke(null, "connectivity") as IBinder
        val stub = Class.forName("android.net.IConnectivityManager\$Stub")
        return stub.getMethod("asInterface", IBinder::class.java).invoke(null, binder)!!
    }

    // The system context's op package is "android"; ConnectivityManager sends
    // getOpPackageName() with every query and ConnectivityService checks it
    // against the calling uid, so the probe must present the target's package.
    private fun packageContext(
        base: Context,
        packageName: String,
    ): Context =
        object : ContextWrapper(base) {
            override fun getOpPackageName(): String = packageName

            override fun getPackageName(): String = packageName

            override fun getAttributionTag(): String? = null
        }
}
