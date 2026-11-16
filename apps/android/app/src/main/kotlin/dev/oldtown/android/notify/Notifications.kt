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
import dev.oldtown.android.files.FileAction
import dev.oldtown.android.files.FileTransfers
import dev.oldtown.android.files.fileActions
import dev.oldtown.android.files.humanSize
import dev.oldtown.android.relay.Connection
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.relay.RelayService
import dev.oldtown.android.relay.Transfer
import dev.oldtown.android.ui.MainActivity
import dev.oldtown.android.ui.channelOf
import dev.oldtown.core.Received

/**
 * Trois canaux : « Connexion » (notification permanente du service,
 * discrète), « Messages » (un élément reçu par notification, groupées) et
 * « Transferts » (progression des fichiers, étape 5).
 */
object Notifications {
    const val CONNECTION_ID = 1
    private const val SUMMARY_ID = 2
    /**
     * Notification imposée par Android au service de premier plan : importance
     * minimale (ni icône dans la barre d'état, ni son, repliée en bas du
     * volet). Nouvel identifiant, car l'importance d'un canal existant ne
     * change plus ; l'ancien (`connection`, importance basse) est supprimé.
     */
    private const val CHANNEL_CONNECTION = "connection_quiet"
    private const val OLD_CHANNEL_CONNECTION = "connection"
    private const val CHANNEL_MESSAGES = "messages"
    private const val CHANNEL_TRANSFERS = "transfers"
    private const val GROUP = "dev.oldtown.android.MESSAGES"

    fun createChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.deleteNotificationChannel(OLD_CHANNEL_CONNECTION)
        manager.createNotificationChannels(
            listOf(
                NotificationChannel(
                    CHANNEL_CONNECTION,
                    context.getString(R.string.channel_connection),
                    NotificationManager.IMPORTANCE_MIN,
                ).apply {
                    description = context.getString(R.string.channel_connection_desc)
                    setShowBadge(false)
                },
                NotificationChannel(
                    CHANNEL_MESSAGES,
                    context.getString(R.string.channel_messages),
                    NotificationManager.IMPORTANCE_HIGH,
                ).apply { description = context.getString(R.string.channel_messages_desc) },
                NotificationChannel(
                    CHANNEL_TRANSFERS,
                    context.getString(R.string.channel_transfers),
                    NotificationManager.IMPORTANCE_LOW,
                ).apply {
                    description = context.getString(R.string.channel_transfers_desc)
                    setShowBadge(false)
                },
            ),
        )
    }

    private fun openApp(context: Context): PendingIntent = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE,
    )

    /** Ouvre l'appli sur la conversation de l'élément (note 0033) : le canal seul, pas le contenu. */
    private fun openConversation(context: Context, item: Received, id: Int): PendingIntent {
        val channel = channelOf(item, RelayHub.state.value.device?.deviceId)
        return PendingIntent.getActivity(
            context,
            id,
            Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
                .putExtra(MainActivity.EXTRA_CHANNEL, channel),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

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
            .setPriority(NotificationCompat.PRIORITY_MIN)
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
        if (item.file != null) {
            showFile(context, item)
            return
        }
        val id = idOf(item.msgId)
        val text = if (item.sensitive) context.getString(R.string.sensitive_hidden) else preview(item.body)
        val builder = NotificationCompat.Builder(context, CHANNEL_MESSAGES)
            .setSmallIcon(R.drawable.ic_stat_oldtown)
            .setContentTitle(item.fromName)
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(if (item.sensitive) text else item.body))
            .setWhen(item.createdAtMs)
            .setShowWhen(true)
            .setContentIntent(openConversation(context, item, id))
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
        trimMessages(context)
    }

    /**
     * Fichier reçu : proposé (« Télécharger », par le service) ou enregistré
     * dans `Download/OldTown` (un appui l'ouvre).
     */
    private fun showFile(context: Context, item: Received) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) return
        val file = item.file ?: return
        val id = idOf(item.msgId)
        val saved = file.savedUri
        val text = if (saved != null) {
            context.getString(R.string.file_saved, file.name)
        } else {
            context.getString(R.string.file_offered, file.name, humanSize(file.size.toLong()))
        }
        val builder = NotificationCompat.Builder(context, CHANNEL_MESSAGES)
            .setSmallIcon(R.drawable.ic_stat_oldtown)
            .setContentTitle(item.fromName)
            .setContentText(text)
            .setWhen(item.createdAtMs)
            .setShowWhen(true)
            .setAutoCancel(true)
            .setGroup(GROUP)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setAllowSystemGeneratedContextualActions(false)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setContentIntent(
                if (saved != null) {
                    PendingIntent.getActivity(
                        context, id, FileTransfers.openIntent(saved, file.mime), PendingIntent.FLAG_IMMUTABLE,
                    )
                } else {
                    openConversation(context, item, id)
                },
            )
        for (action in fileActions(saved != null)) {
            val (label, intent) = when (action) {
                FileAction.DOWNLOAD -> R.string.action_download to PendingIntent.getForegroundService(
                    context,
                    id,
                    Intent(context, RelayService::class.java)
                        .setAction(RelayService.ACTION_DOWNLOAD)
                        .putExtra(RelayService.EXTRA_MSG_ID, item.msgId),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                )
                FileAction.OPEN -> R.string.action_open to PendingIntent.getActivity(
                    context, id, FileTransfers.openIntent(saved ?: continue, file.mime), PendingIntent.FLAG_IMMUTABLE,
                )
                FileAction.SHARE -> R.string.action_share to PendingIntent.getActivity(
                    context, id + 1, FileTransfers.shareIntent(saved ?: continue, file.mime), PendingIntent.FLAG_IMMUTABLE,
                )
            }
            builder.addAction(0, context.getString(label), intent)
        }
        val manager = NotificationManagerCompat.from(context)
        manager.notify(id, builder.build())
        trimMessages(context)
    }

    /** Progression d'un transfert (notification silencieuse, en cours). */
    fun transferProgress(context: Context, t: Transfer) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) return
        val title = context.getString(if (t.upload) R.string.transfer_up else R.string.transfer_down, t.name)
        val percent = if (t.total > 0) (t.done * 100 / t.total).toInt() else 0
        NotificationManagerCompat.from(context).notify(
            transferId(t.id),
            NotificationCompat.Builder(context, CHANNEL_TRANSFERS)
                .setSmallIcon(R.drawable.ic_stat_oldtown)
                .setContentTitle(title)
                .setContentText("${humanSize(t.done)} / ${humanSize(t.total)}")
                .setProgress(100, percent, t.total <= 0)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setSilent(true)
                .setContentIntent(openApp(context))
                .setCategory(NotificationCompat.CATEGORY_PROGRESS)
                .build(),
        )
    }

    fun transferEnd(context: Context, id: String) {
        NotificationManagerCompat.from(context).cancel(transferId(id))
    }

    /** Transfert en échec : il se relance depuis l'appli (ou en repartageant). */
    fun transferFailed(context: Context, t: Transfer) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) return
        val title = context.getString(if (t.upload) R.string.transfer_up_failed else R.string.transfer_down_failed, t.name)
        NotificationManagerCompat.from(context).notify(
            transferId(t.id),
            NotificationCompat.Builder(context, CHANNEL_TRANSFERS)
                .setSmallIcon(R.drawable.ic_stat_oldtown)
                .setContentTitle(title)
                .setContentText(t.error)
                .setStyle(NotificationCompat.BigTextStyle().bigText(t.error))
                .setAutoCancel(true)
                .setContentIntent(openApp(context))
                .build(),
        )
    }

    private fun transferId(id: String): Int = 500_000_000 + (id.hashCode() and 0x0fffffff)

    /** Messages affichés : (identifiant, date de publication), sans le résumé du groupe. */
    private fun shownMessages(context: Context): List<Pair<Int, Long>> =
        context.getSystemService(NotificationManager::class.java).activeNotifications
            .filter { it.notification.channelId == CHANNEL_MESSAGES && it.id != SUMMARY_ID }
            .map { it.id to it.postTime }

    /** Au plus [MAX_SHOWN_MESSAGES] : les plus anciennes laissent la place. */
    private fun trimMessages(context: Context) {
        val manager = NotificationManagerCompat.from(context)
        for (id in oldestBeyond(shownMessages(context), MAX_SHOWN_MESSAGES)) manager.cancel(id)
    }

    /**
     * Écran principal ouvert : les éléments sont dans la liste, les
     * notifications de messages n'ont plus lieu d'être (étape 4, T8b).
     */
    fun clearMessages(context: Context) {
        val manager = NotificationManagerCompat.from(context)
        for ((id, _) in shownMessages(context)) manager.cancel(id)
        manager.cancel(SUMMARY_ID)
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
