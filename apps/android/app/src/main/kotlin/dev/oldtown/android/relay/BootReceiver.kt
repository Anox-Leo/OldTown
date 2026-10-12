package dev.oldtown.android.relay

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat

/**
 * Démarrage du téléphone ou mise à jour de l'appli : relance le service si
 * l'appareil est appairé (le service s'arrête seul sinon). `specialUse` peut
 * démarrer depuis `BOOT_COMPLETED` (vérifié dans la doc Android, note 0022).
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED &&
            intent.action != Intent.ACTION_MY_PACKAGE_REPLACED
        ) return
        ContextCompat.startForegroundService(context, Intent(context, RelayService::class.java))
    }
}
