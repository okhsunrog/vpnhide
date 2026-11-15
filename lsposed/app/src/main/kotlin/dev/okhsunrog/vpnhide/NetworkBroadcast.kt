package dev.okhsunrog.vpnhide

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.Handler

/** The platform fills the network extras into the package-scoped broadcast. */
internal fun networkBroadcastIntent(
    context: Context,
    action: String,
    requestCode: Int,
    intentFlags: Int = 0,
): PendingIntent {
    val flags = PendingIntent.FLAG_UPDATE_CURRENT or if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
    val intent = Intent(action).setPackage(context.packageName).addFlags(intentFlags)
    return PendingIntent.getBroadcast(context, requestCode, intent, flags)
}

internal fun registerPrivateNetworkReceiver(
    context: Context,
    receiver: BroadcastReceiver,
    filter: IntentFilter,
    scheduler: Handler? = null,
) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        context.registerReceiver(receiver, filter, null, scheduler, Context.RECEIVER_NOT_EXPORTED)
    } else {
        context.registerReceiver(receiver, filter, null, scheduler)
    }
}
