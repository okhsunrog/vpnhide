package dev.okhsunrog.vpnhide

import android.Manifest
import android.annotation.SuppressLint
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import dev.okhsunrog.vpnhide.settings.AppSettings
import dev.okhsunrog.vpnhide.settings.SettingsRepository
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import java.util.concurrent.TimeUnit

private const val TAG = "VpnHide-Update"
private const val UPDATE_WORK_NAME = "vpnhide_background_update_check"
private const val UPDATE_NOTIFICATION_CHANNEL_ID = "vpnhide_updates"
private const val UPDATE_NOTIFICATION_ID = 1001
private const val UPDATE_PREFS_NAME = "vpnhide_prefs"
private const val KEY_LAST_NOTIFIED_UPDATE_VERSION = "last_notified_update_version"

internal object BackgroundUpdateChecks {
    fun sync(
        context: Context,
        settings: AppSettings,
    ) {
        val appContext = context.applicationContext
        if (settings.backgroundUpdateChecksConfigured && settings.backgroundUpdateChecksEnabled) {
            schedule(appContext)
        } else {
            cancel(appContext)
        }
    }

    fun schedule(context: Context) {
        val request =
            PeriodicWorkRequestBuilder<BackgroundUpdateCheckWorker>(1, TimeUnit.DAYS)
                .setConstraints(
                    Constraints
                        .Builder()
                        .setRequiredNetworkType(NetworkType.CONNECTED)
                        .build(),
                ).build()

        WorkManager
            .getInstance(context.applicationContext)
            .enqueueUniquePeriodicWork(
                UPDATE_WORK_NAME,
                ExistingPeriodicWorkPolicy.UPDATE,
                request,
            )
    }

    fun cancel(context: Context) {
        WorkManager
            .getInstance(context.applicationContext)
            .cancelUniqueWork(UPDATE_WORK_NAME)
    }
}

internal fun shouldRequestUpdateNotificationPermission(context: Context): Boolean =
    Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
        PackageManager.PERMISSION_GRANTED

internal fun canPostUpdateNotifications(context: Context): Boolean =
    !shouldRequestUpdateNotificationPermission(context) &&
        NotificationManagerCompat.from(context).areNotificationsEnabled()

class BackgroundUpdateCheckWorker(
    appContext: Context,
    params: WorkerParameters,
) : Worker(appContext, params) {
    override fun doWork(): Result {
        if (!backgroundChecksEnabled()) return Result.success()

        return when (val result = checkForUpdate(BuildConfig.VERSION_NAME)) {
            is UpdateCheckResult.Available -> {
                notifyIfNeeded(applicationContext, result.info)
                Result.success()
            }

            UpdateCheckResult.UpToDate -> {
                Result.success()
            }

            UpdateCheckResult.Failed -> {
                Result.success()
            }
        }
    }

    private fun backgroundChecksEnabled(): Boolean =
        runCatching {
            runBlocking {
                SettingsRepository(applicationContext).settings.first().backgroundUpdateChecksEnabled
            }
        }.getOrDefault(false)
}

private fun notifyIfNeeded(
    context: Context,
    info: UpdateInfo,
) {
    if (!canPostUpdateNotifications(context)) {
        VpnHideLog.d(TAG, "Update ${info.latestVersion} available, but notifications are disabled")
        return
    }
    val prefs = context.getSharedPreferences(UPDATE_PREFS_NAME, Context.MODE_PRIVATE)
    if (prefs.getString(KEY_LAST_NOTIFIED_UPDATE_VERSION, null) == info.latestVersion) return
    if (showUpdateNotification(context, info)) {
        prefs
            .edit()
            .putString(KEY_LAST_NOTIFIED_UPDATE_VERSION, info.latestVersion)
            .apply()
    }
}

@SuppressLint("MissingPermission")
private fun showUpdateNotification(
    context: Context,
    info: UpdateInfo,
): Boolean {
    ensureUpdateNotificationChannel(context)
    val intent =
        Intent(Intent.ACTION_VIEW, Uri.parse(info.downloadUrl))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    val pendingIntent =
        PendingIntent.getActivity(
            context,
            0,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    val notification =
        NotificationCompat
            .Builder(context, UPDATE_NOTIFICATION_CHANNEL_ID)
            .setSmallIcon(R.mipmap.ic_launcher_monochrome)
            .setContentTitle(context.getString(R.string.update_notification_title))
            .setContentText(context.getString(R.string.update_notification_body, info.latestVersion))
            .setStyle(
                NotificationCompat.BigTextStyle().bigText(
                    context.getString(R.string.update_notification_body, info.latestVersion),
                ),
            ).setContentIntent(pendingIntent)
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .build()

    return runCatching {
        NotificationManagerCompat.from(context).notify(UPDATE_NOTIFICATION_ID, notification)
        true
    }.getOrElse { error ->
        VpnHideLog.d(TAG, "Failed to show update notification: ${error.message}")
        false
    }
}

private fun ensureUpdateNotificationChannel(context: Context) {
    val manager = context.getSystemService(NotificationManager::class.java)
    val channel =
        NotificationChannel(
            UPDATE_NOTIFICATION_CHANNEL_ID,
            context.getString(R.string.update_notification_channel_name),
            NotificationManager.IMPORTANCE_DEFAULT,
        ).apply {
            description = context.getString(R.string.update_notification_channel_description)
        }
    manager.createNotificationChannel(channel)
}
