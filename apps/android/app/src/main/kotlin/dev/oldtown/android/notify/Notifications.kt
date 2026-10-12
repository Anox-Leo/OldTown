package dev.oldtown.android.notify

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.core.net.toUri
import dev.oldtown.android.R
import dev.oldtown.android.relay.Connection
import dev.oldtown.android.ui.MainActivity
import dev.oldtown.core.Received

/**
 * Deux canaux : « Connexion » (notification permanente du service, discrète)
 * et « Messages » (un élément reçu par notification, groupées).
 */
object Notifications {
    const val CONNECTION_ID = 1
    private const val SUMMARY_ID = 2
    private const val CHANNEL_CONNECTION = "connection"
    private const val CHANNEL_MESSAGES = "messages"
    private const val GROUP = "dev.oldtown.android.MESSAGES"

    fun createChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannels(
            listOf(
                NotificationChannel(
                    CHANNEL_CONNECTION,
                    context.getString(R.string.channel_connection),
                    NotificationManager.IMPORTANCE_LOW,
                ).apply {
                    description = context.getString(R.string.channel_connection_desc)
                    setShowBadge(false)
                },
                NotificationChannel(
                    CHANNEL_MESSAGES,
                    context.getString(R.string.channel_messages),
                    NotificationManager.IMPORTANCE_HIGH,
                ).apply { description = context.getString(R.string.channel_messages_desc) },
            ),
        )
    }

    private fun openApp(context: Context): PendingIntent = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE,
    )

    /** Texte de la notification permanente pour un état de connexion. */
    fun statusText(context: Context, connection: Connection, waitingSponsor: Boolean): String =
        context.getString(
            when {
                waitingSponsor && connection is Connection.Connected -> R.string.status_waiting_sponsor
                connection is Connection.Connected -> R.string.status_connected
                connection is Connection.Reconnecting && connection.failures >= UNREACHABLE_AFTER ->
                    R.string.status_unreachable
                connection is Connection.Reconnecting -> R.string.status_reconnecting
                connection is Connection.AuthFailed -> R.string.status_auth_failed
                else -> R.string.status_starting
            },
        )

    fun connection(context: Context, connection: Connection, waitingSponsor: Boolean): Notification =
        NotificationCompat.Builder(context, CHANNEL_CONNECTION)
            .setSmallIcon(R.drawable.ic_stat_oldtown)
            .setContentTitle(statusText(context, connection, waitingSponsor))
            .setContentIntent(openApp(context))
            .setOngoing(true)
            .setSilent(true)
            .setShowWhen(false)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()

    fun updateConnection(context: Context, connection: Connection, waitingSponsor: Boolean) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) return
        NotificationManagerCompat.from(context)
            .notify(CONNECTION_ID, connection(context, connection, waitingSponsor))
    }

    /** Une notification par élément reçu, avec ses actions. */
    fun showMessage(context: Context, item: Received) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) return
        val id = idOf(item.msgId)
        val text = if (item.sensitive) context.getString(R.string.sensitive_hidden) else preview(item.body)
        val builder = NotificationCompat.Builder(context, CHANNEL_MESSAGES)
            .setSmallIcon(R.drawable.ic_stat_oldtown)
            .setContentTitle(item.fromName)
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(if (item.sensitive) text else item.body))
            .setWhen(item.createdAtMs)
            .setShowWhen(true)
            .setContentIntent(openApp(context))
            .setAutoCancel(true)
            .setGroup(GROUP)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            // Nos actions suffisent : pas d'« Ouvrir le lien » ajouté par Android.
            .setAllowSystemGeneratedContextualActions(false)
            .setVisibility(
                if (item.sensitive) NotificationCompat.VISIBILITY_SECRET else NotificationCompat.VISIBILITY_PRIVATE,
            )
        for (action in actionsFor(item.kind, item.body)) {
            builder.addAction(0, context.getString(label(action)), pending(context, action, item, id))
        }
        val manager = NotificationManagerCompat.from(context)
        manager.notify(id, builder.build())
        manager.notify(
            SUMMARY_ID,
            NotificationCompat.Builder(context, CHANNEL_MESSAGES)
                .setSmallIcon(R.drawable.ic_stat_oldtown)
                .setGroup(GROUP)
                .setGroupSummary(true)
                .setAutoCancel(true)
                .setSilent(true)
                .setContentIntent(openApp(context))
                .build(),
        )
    }

    fun cancel(context: Context, notificationId: Int) {
        NotificationManagerCompat.from(context).cancel(notificationId)
    }

    private fun label(action: MessageAction) = when (action) {
        MessageAction.COPY -> R.string.action_copy
        MessageAction.OPEN -> R.string.action_open
        MessageAction.SHARE -> R.string.action_share
    }

    private fun pending(context: Context, action: MessageAction, item: Received, id: Int): PendingIntent =
        when (action) {
            // Copier passe par un récepteur : le texte ne quitte pas l'appli,
            // il est relu dans l'historique par son msg_id.
            MessageAction.COPY -> PendingIntent.getBroadcast(
                context,
                id,
                Intent(context, ActionReceiver::class.java)
                    .setAction(ActionReceiver.ACTION_COPY)
                    .putExtra(ActionReceiver.EXTRA_MSG_ID, item.msgId)
                    .putExtra(ActionReceiver.EXTRA_NOTIFICATION_ID, id),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            MessageAction.OPEN -> PendingIntent.getActivity(
                context,
                id,
                Intent(Intent.ACTION_VIEW, item.body.trim().toUri())
                    .addCategory(Intent.CATEGORY_BROWSABLE)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                PendingIntent.FLAG_IMMUTABLE,
            )
            MessageAction.SHARE -> PendingIntent.getActivity(
                context,
                id,
                shareIntent(item.body),
                PendingIntent.FLAG_IMMUTABLE,
            )
        }

    /** Sélecteur Partager pour un texte reçu. */
    fun shareIntent(text: String): Intent = Intent.createChooser(
        Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text),
        null,
    ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    /** Identifiant de notification stable pour un `msg_id`. */
    fun idOf(msgId: String): Int = 1_000 + (msgId.hashCode() and 0x3fffffff)

    /** Échecs de connexion de suite avant de soupçonner Tailscale. */
    const val UNREACHABLE_AFTER = 3
}
