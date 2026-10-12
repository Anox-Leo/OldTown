package dev.oldtown.android.ui

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.NotificationManagerCompat
import androidx.core.net.toUri

/** Réglages du téléphone dont dépend la réception. */
object SystemState {
    fun notificationsAllowed(context: Context) =
        NotificationManagerCompat.from(context).areNotificationsEnabled()

    fun batteryExempt(context: Context) =
        context.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(context.packageName)

    /**
     * Demande l'exemption d'optimisation de batterie : sans elle, Doze coupe
     * le réseau de l'appli et les messages n'arrivent qu'au réveil. Autorisé
     * hors Play Store (étape 3).
     */
    @SuppressLint("BatteryLife")
    fun requestBatteryExemption(context: Context) {
        context.startActivity(
            Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, "package:${context.packageName}".toUri()),
        )
    }

    fun openNotificationSettings(context: Context) {
        context.startActivity(
            Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName),
        )
    }
}
