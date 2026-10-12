package dev.oldtown.android.ui

import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import androidx.lifecycle.compose.LifecycleResumeEffect
import dev.oldtown.android.notify.Clipboard
import dev.oldtown.android.notify.MessageAction
import dev.oldtown.android.notify.Notifications
import dev.oldtown.android.notify.actionsFor
import dev.oldtown.android.relay.Connection
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.components.ButtonKind
import dev.oldtown.android.ui.components.Divider
import dev.oldtown.android.ui.components.GhostIcon
import dev.oldtown.android.ui.components.OtButton
import dev.oldtown.android.ui.components.OtCard
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.components.OtTextField
import dev.oldtown.android.ui.components.SectionLabel
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.android.ui.theme.OtShapes
import dev.oldtown.core.ContentKind
import dev.oldtown.core.OtException
import dev.oldtown.core.Received
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.launch

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

@Composable
fun HomeScreen(hub: HubState, open: (Screen) -> Unit) {
    val context = LocalContext.current
    var selected by remember { mutableStateOf<Received?>(null) }
    var notifications by remember { mutableStateOf(true) }
    var battery by remember { mutableStateOf(true) }
    LifecycleResumeEffect(Unit) {
        notifications = SystemState.notificationsAllowed(context)
        battery = SystemState.batteryExempt(context)
        onPauseOrDispose {}
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
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 4.dp, bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item { Alerts(hub, notifications, battery) }
            item { Composer(hub) }
            item { SectionLabel("Reçus", Modifier.padding(top = 8.dp)) }
            if (hub.history.isEmpty()) {
                item { EmptyHistory() }
            } else {
                item {
                    OtCard(padding = PaddingValues(0.dp)) {
                        hub.history.forEachIndexed { i, item ->
                            if (i > 0) Divider()
                            ReceivedRow(item, onOpen = { selected = item })
                        }
                    }
                }
            }
        }
    }
    selected?.let { ItemSheet(it, onDismiss = { selected = null }) }
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

@Composable
private fun Composer(hub: HubState) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var text by rememberSaveable { mutableStateOf("") }
    var to by rememberSaveable { mutableStateOf<String?>(null) }
    var menu by remember { mutableStateOf(false) }
    var sending by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<Pair<Boolean, String>?>(null) }
    val others = hub.group?.members?.filterNot { it.isSelf }.orEmpty()
    val toName = others.firstOrNull { it.deviceId == to }?.name ?: "Tous mes appareils"

    OtCard {
        OtTextField(
            value = text,
            onValueChange = {
                text = it
                result = null
            },
            placeholder = "Texte ou lien à envoyer…",
            singleLine = false,
            minLines = 3,
        )
        Spacer(Modifier.height(10.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box {
                Row(
                    Modifier
                        .clip(OtShapes.pill)
                        .background(Ot.colors.groupedCard)
                        .clickable(enabled = others.isNotEmpty()) { menu = true }
                        .padding(horizontal = 12.dp, vertical = 7.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text("À : ", style = MaterialTheme.typography.labelMedium, color = Ot.colors.muted)
                    Text(toName, style = MaterialTheme.typography.labelMedium, color = Ot.colors.foreground)
                }
                DropdownMenu(menu, onDismissRequest = { menu = false }, containerColor = Ot.colors.groupedCard) {
                    DropdownMenuItem(text = { Text("Tous mes appareils") }, onClick = { to = null; menu = false })
                    others.forEach { m ->
                        DropdownMenuItem(text = { Text(m.name) }, onClick = { to = m.deviceId; menu = false })
                    }
                }
            }
            Spacer(Modifier.weight(1f))
            GhostIcon(OtIcons.Copy, "Coller", {
                val clip = context.getSystemService(ClipboardManager::class.java).primaryClip
                clip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.let { text = it.toString() }
            }, tint = Ot.colors.muted)
            Spacer(Modifier.width(6.dp))
            Box(
                Modifier
                    .size(38.dp)
                    .clip(CircleShape)
                    .background(if (text.isBlank() || sending) Ot.colors.groupedCard else Ot.colors.primary)
                    .clickable(enabled = text.isNotBlank() && !sending && hub.group != null) {
                        sending = true
                        scope.launch {
                            result = try {
                                RelayHub.send(context, text, listOfNotNull(to))
                                text = ""
                                true to if (to == null) "Envoyé à tous vos appareils" else "Envoyé à $toName"
                            } catch (e: OtException) {
                                false to (e.message ?: "envoi impossible")
                            } catch (_: TimeoutCancellationException) {
                                false to "le service ne répond pas"
                            }
                            sending = false
                        }
                    },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    OtIcons.Send,
                    contentDescription = "Envoyer",
                    tint = if (text.isBlank() || sending) Ot.colors.muted else Ot.colors.onPrimary,
                    modifier = Modifier.size(18.dp),
                )
            }
        }
        result?.let { (ok, message) ->
            Spacer(Modifier.height(10.dp))
            Text(
                message,
                style = MaterialTheme.typography.bodySmall,
                color = if (ok) Ot.colors.success else Ot.colors.dangerForeground,
            )
        }
    }
}

@Composable
private fun EmptyHistory() {
    OtCard {
        Text("Rien reçu pour l'instant.", style = MaterialTheme.typography.bodyMedium, color = Ot.colors.foreground)
        Spacer(Modifier.height(4.dp))
        Text(
            "Depuis un ordinateur du groupe : ot send \"bonjour\". Les 50 derniers éléments reçus restent ici.",
            style = MaterialTheme.typography.bodySmall,
            color = Ot.colors.muted,
        )
    }
}

fun relativeTime(ms: Long): String {
    val now = System.currentTimeMillis()
    return if (now - ms < DateUtils.MINUTE_IN_MILLIS) {
        "à l'instant"
    } else {
        DateUtils.getRelativeTimeSpanString(ms, now, DateUtils.MINUTE_IN_MILLIS).toString()
    }
}

@Composable
private fun ReceivedRow(item: Received, onOpen: () -> Unit) {
    val context = LocalContext.current
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onOpen).padding(start = 16.dp, top = 12.dp, bottom = 12.dp, end = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(item.fromName, style = MaterialTheme.typography.labelMedium, color = Ot.colors.foreground)
                Text(
                    "  ·  " + relativeTime(item.createdAtMs),
                    style = MaterialTheme.typography.labelMedium,
                    color = Ot.colors.muted,
                )
            }
            Spacer(Modifier.height(3.dp))
            Text(
                item.body.trim(),
                style = MaterialTheme.typography.bodyMedium,
                color = if (item.kind == ContentKind.LINK) Ot.colors.primaryText else Ot.colors.foreground,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
        }
        GhostIcon(OtIcons.Copy, "Copier", { Clipboard.copy(context, item.body, item.sensitive) }, tint = Ot.colors.muted)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ItemSheet(item: Received, onDismiss: () -> Unit) {
    val context = LocalContext.current
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        shape = OtShapes.sheet,
        containerColor = Ot.colors.card,
        scrimColor = Ot.colors.backdrop,
    ) {
        Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 28.dp)) {
            Text(
                "${item.fromName}  ·  ${relativeTime(item.createdAtMs)}",
                style = MaterialTheme.typography.labelMedium,
                color = Ot.colors.muted,
            )
            Spacer(Modifier.height(10.dp))
            androidx.compose.foundation.text.selection.SelectionContainer {
                Text(
                    item.body,
                    style = MaterialTheme.typography.bodyLarge,
                    color = if (item.kind == ContentKind.LINK) Ot.colors.primaryText else Ot.colors.foreground,
                )
            }
            Spacer(Modifier.height(20.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (action in actionsFor(item.kind, item.body)) {
                    when (action) {
                        MessageAction.COPY -> OtButton("Copier", {
                            Clipboard.copy(context, item.body, item.sensitive)
                            onDismiss()
                        }, Modifier.weight(1f), icon = OtIcons.Copy)
                        MessageAction.OPEN -> OtButton("Ouvrir", {
                            openLink(context, item.body)
                            onDismiss()
                        }, Modifier.weight(1f), kind = ButtonKind.Secondary, icon = OtIcons.External)
                        MessageAction.SHARE -> OtButton("Partager", {
                            context.startActivity(Notifications.shareIntent(item.body))
                            onDismiss()
                        }, Modifier.weight(1f), kind = ButtonKind.Secondary, icon = OtIcons.Share)
                    }
                }
            }
        }
    }
}

private fun openLink(context: Context, link: String) {
    context.startActivity(
        Intent(Intent.ACTION_VIEW, link.trim().toUri()).addCategory(Intent.CATEGORY_BROWSABLE),
    )
}
