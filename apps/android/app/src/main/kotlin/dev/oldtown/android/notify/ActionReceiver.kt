package dev.oldtown.android.notify

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import dev.oldtown.android.OldTownApp
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.core.OtException
import dev.oldtown.core.history
import kotlinx.coroutines.launch

/** Action « Copier » d'une notification : écrit l'élément dans le presse-papiers. */
class ActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_COPY) return
        val msgId = intent.getStringExtra(EXTRA_MSG_ID) ?: return
        val notificationId = intent.getIntExtra(EXTRA_NOTIFICATION_ID, 0)
        val pending = goAsync()
        OldTownApp.scope.launch {
            try {
                val item = RelayHub.state.value.history.firstOrNull { it.msgId == msgId }
                    ?: history(RelayHub.store(context)).firstOrNull { it.msgId == msgId }
                if (item != null) {
                    Clipboard.copy(context, item.body, item.sensitive)
                    Notifications.cancel(context, notificationId)
                } else {
                    Log.w("OldTown", "élément à copier introuvable (historique effacé ?)")
                }
            } catch (e: OtException) {
                Log.w("OldTown", "copie impossible : ${e.message}")
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION_COPY = "dev.oldtown.android.action.COPY"
        const val EXTRA_MSG_ID = "msg_id"
        const val EXTRA_NOTIFICATION_ID = "notification_id"
    }
}
