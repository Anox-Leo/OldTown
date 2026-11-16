package dev.oldtown.android.ui

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import dev.oldtown.android.notify.Notifications
import dev.oldtown.android.relay.Connection
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.relay.Transfer
import dev.oldtown.android.ui.components.Avatar
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.components.Divider
import dev.oldtown.android.ui.components.GhostIcon
import dev.oldtown.android.ui.components.OtCard
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.core.OtException
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Présence relue auprès du relais tant que l'écran est au premier plan. */
private const val PRESENCE_EVERY_MS = 15_000L

/** Libellé et couleur de l'état de connexion. */
@Composable
fun connectionLabel(hub: HubState): Pair<String, Color> {
    val c = Ot.colors
    val waiting = hub.group == null
    return when (val conn = hub.connection) {
        is Connection.Connected -> if (waiting) "En attente du parrain" to c.warningForeground else "Connecté" to c.success
        is Connection.Reconnecting ->
            if (conn.failures >= Notifications.UNREACHABLE_AFTER) "Injoignable" to c.dangerForeground
            else "Reconnexion…" to c.warningForeground
        is Connection.AuthFailed -> "Refusé" to c.dangerForeground
        Connection.Connecting -> "Connexion…" to c.muted
        Connection.Stopped -> "Arrêté" to c.muted
    }
}

/**
 * Accueil (note 0033, comme l'extension, note 0031) : les bandeaux, puis la
 * liste des conversations, « Tous mes appareils » en premier, puis un
 * appareil par ligne, du plus récemment actif au moins récent. Toucher une
 * ligne ouvre la conversation.
 */
@Composable
fun HomeScreen(hub: HubState, open: (Screen) -> Unit) {
    val context = LocalContext.current
    var notifications by remember { mutableStateOf(true) }
    var battery by remember { mutableStateOf(true) }
    LifecycleResumeEffect(Unit) {
        notifications = SystemState.notificationsAllowed(context)
        battery = SystemState.batteryExempt(context)
        onPauseOrDispose {}
    }
    val presence = rememberPresence(hub.connection)
    val me = hub.device?.deviceId
    val list = remember(hub.group, hub.history, me) {
        conversations(hub.group?.members.orEmpty(), hub.history, me)
    }

    Column(Modifier.fillMaxSize()) {
        Row(
            Modifier.fillMaxWidth().height(56.dp).padding(start = 20.dp, end = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("OldTown", style = MaterialTheme.typography.titleMedium, color = Ot.colors.foreground)
            Spacer(Modifier.weight(1f))
            GhostIcon(OtIcons.Users, "Groupe", { open(Screen.Group) })
            GhostIcon(OtIcons.Settings, "Réglages", { open(Screen.Settings) })
        }

        LazyColumn(
            Modifier.fillMaxSize().testTag("conversations"),
            contentPadding = PaddingValues(top = 4.dp, bottom = 24.dp),
        ) {
            item {
                Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Alerts(hub, notifications, battery)
                    if (hub.transfers.isNotEmpty()) Transfers(hub.transfers)
                }
            }
            items(list, key = { "c:" + it.id }) { c ->
                ConversationRow(
                    c,
                    online = presence?.get(c.id) == true,
                    divider = c.id != ALL_CHANNEL,
                    onClick = { open(Screen.Conversation(c.id)) },
                )
            }
        }
    }
}

/**
 * Présence des appareils d'après le relais : lue à l'affichage puis toutes
 * les 15 s, tant que l'écran est au premier plan et la connexion établie ;
 * `null` (inconnue) sinon.
 */
@Composable
internal fun rememberPresence(connection: Connection): Map<String, Boolean>? {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var presence by remember { mutableStateOf<Map<String, Boolean>?>(null) }
    val connected = connection is Connection.Connected
    LifecycleResumeEffect(connected) {
        val job = if (connected) {
            scope.launch {
                while (true) {
                    presence = loadPresence(context)
                    delay(PRESENCE_EVERY_MS)
                }
            }
        } else {
            presence = null
            null
        }
        onPauseOrDispose { job?.cancel() }
    }
    return presence
}

private suspend fun loadPresence(context: Context): Map<String, Boolean>? = try {
    RelayHub.client(context).devices().associate { it.deviceId to it.online }
} catch (_: OtException) {
    // Relais injoignable entre deux : l'état de la connexion le dira.
    null
} catch (_: TimeoutCancellationException) {
    null
}

/** Ligne d'une conversation : avatar, nom et heure, aperçu du dernier élément. */
@Composable
private fun ConversationRow(c: Conversation, online: Boolean, divider: Boolean, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp).testTag("conversation"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Avatar(c.kind, c.color, online = online)
        Spacer(Modifier.width(12.dp))
        // Séparateur à la WhatsApp : sous le texte, pas sous l'avatar.
        Column(Modifier.weight(1f)) {
            if (divider) Divider()
            Column(Modifier.padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        c.name,
                        style = MaterialTheme.typography.bodyLarge.copy(fontWeight = FontWeight.SemiBold),
                        color = Ot.colors.foreground,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f).testTag("conversation-name"),
                    )
                    c.last?.let {
                        Spacer(Modifier.width(6.dp))
                        Text(shortTime(it.createdAtMs), style = MaterialTheme.typography.labelSmall, color = Ot.colors.muted)
                    }
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    val last = c.last
                    if (last?.file != null) {
                        Icon(OtIcons.Paperclip, null, tint = Ot.colors.muted, modifier = Modifier.size(14.dp))
                        Spacer(Modifier.width(4.dp))
                    }
                    Text(
                        if (last != null) preview(last, c.id) else "Aucun message",
                        style = MaterialTheme.typography.bodySmall,
                        color = Ot.colors.muted,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

@Composable
private fun Alerts(hub: HubState, notifications: Boolean, battery: Boolean) {
    val context = LocalContext.current
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val conn = hub.connection
        if (hub.group == null) {
            Banner(
                "Appairage en cours : le parrain doit être en ligne pour envoyer le groupe à ce téléphone.",
                BannerKind.Warning,
            )
        }
        if (conn is Connection.AuthFailed) {
            Banner("Le relais refuse ce téléphone (révoqué ?) : ${conn.message}", BannerKind.Danger)
        }
        if (conn is Connection.Reconnecting && conn.failures >= Notifications.UNREACHABLE_AFTER) {
            Banner(
                "Relais injoignable. Tailscale est-il connecté sur ce téléphone ?",
                BannerKind.Danger,
                "Réessayer",
            ) { RelayHub.restart(context) }
        }
        if (!notifications) {
            Banner("Notifications désactivées : rien ne s'affichera à la réception.", BannerKind.Warning, "Activer") {
                SystemState.openNotificationSettings(context)
            }
        }
        if (!battery) {
            Banner("Optimisation de batterie active : les messages peuvent attendre le réveil.", BannerKind.Info, "Corriger") {
                SystemState.requestBatteryExemption(context)
            }
        }
    }
}

/** Fichiers en cours d'envoi ou de réception. */
@Composable
private fun Transfers(transfers: List<Transfer>) {
    OtCard {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            for (t in transfers) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        (if (t.upload) "↑ " else "↓ ") + t.name,
                        style = MaterialTheme.typography.bodySmall,
                        color = Ot.colors.foreground,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    Text(
                        if (t.total > 0) "${t.done * 100 / t.total} %" else "",
                        style = MaterialTheme.typography.labelMedium,
                        color = Ot.colors.muted,
                    )
                }
                if (t.error != null) {
                    Text(t.error, style = MaterialTheme.typography.bodySmall, color = Ot.colors.dangerForeground)
                } else {
                    androidx.compose.material3.LinearProgressIndicator(
                        progress = { if (t.total > 0) t.done.toFloat() / t.total else 0f },
                        modifier = Modifier.fillMaxWidth(),
                        color = Ot.colors.primary,
                        trackColor = Ot.colors.groupedCard,
                    )
                }
            }
        }
    }
}
