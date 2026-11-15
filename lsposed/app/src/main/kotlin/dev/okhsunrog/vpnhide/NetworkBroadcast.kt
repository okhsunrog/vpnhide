package dev.okhsunrog.vpnhide

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build

/** The platform fills the network extras into the package-scoped broadcast. */
internal fun networkBroadcastIntent(
    context: Context,
    action: String,
    requestCode: Int,
): PendingIntent {
    val flags = PendingIntent.FLAG_UPDATE_CURRENT or if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
    return PendingIntent.getBroadcast(context, requestCode, Intent(action).setPackage(context.packageName), flags)
}

internal fun registerPrivateNetworkReceiver(
    context: Context,
    receiver: BroadcastReceiver,
    filter: IntentFilter,
) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
    } else {
        context.registerReceiver(receiver, filter)
    }
}
