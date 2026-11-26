package dev.oldtown.android.ui

import android.util.Log
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
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
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.oldtown.android.files.FileTransfers
import dev.oldtown.android.files.humanSize
import dev.oldtown.android.relay.Connection
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.relay.Transfer
import dev.oldtown.android.ui.components.Avatar
import dev.oldtown.android.ui.components.GhostIcon
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.components.OtTextField
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.core.ContentKind
import dev.oldtown.core.OtException
import dev.oldtown.core.Received
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.launch

private data class Header(
    val name: String,
    val kind: DeviceKind,
    val color: Long,
    val initials: String = "",
    val online: Boolean? = null,
    val readOnly: Boolean = false,
    val outdated: Boolean = false,
)

private fun headerOf(hub: HubState, channel: String): Header = when (val t = parseChannel(channel)) {
    is ChannelTarget.Mine -> {
        val member = hub.group?.members?.firstOrNull { !it.isSelf && it.deviceId == channel }
        if (member == null) {
            Header("Appareil retiré", DeviceKind.UNKNOWN, avatarColor(channel), readOnly = true)
        } else {
            val profile = hub.me?.devices?.firstOrNull { it.deviceId == member.deviceId }
            Header(member.name, deviceKind(profile?.kind), avatarColor(member.deviceId))
        }
    }
    is ChannelTarget.Person -> {
        val person = hub.people.firstOrNull { it.userId == t.user }
        val device = person?.devices?.firstOrNull { it.deviceId == t.device }
        val name = person?.name ?: "Personne inconnue"
        when {
            device != null -> Header(
                "${device.name} · $name",
                deviceKind(device.kind),
                avatarColor(device.deviceId),
                online = device.online,
                outdated = !device.upToDate,
            )
            else -> Header(
                name,
                DeviceKind.PERSON,
                avatarColor(t.user),
                initials = initials(name),
                online = person?.devices?.any { it.online } ?: false,
                readOnly = person?.isService == true,
            )
        }
    }
}

private fun senderName(hub: HubState, item: Received): String {
    if (item.fromUser == null) return item.fromName
    return hub.people.firstOrNull { it.userId == item.fromUser }?.devices?.firstOrNull { it.deviceId == item.fromId }?.name
        ?: item.fromName
}

private val SWIPE_BACK = 96.dp

@Composable
fun ConversationScreen(hub: HubState, channel: String, onBack: () -> Unit, onOpened: suspend (channel: String) -> Unit) {
    val context = LocalContext.current
    val presence = rememberPresence(hub.connection)
    val me = hub.device?.deviceId
    val others = hub.group?.members?.filterNot { it.isSelf }.orEmpty()
    val header = headerOf(hub, channel)
    val target = parseChannel(channel)
    val device = (target as? ChannelTarget.Mine)?.device
    val person = (target as? ChannelTarget.Person)?.takeIf { it.device == null }
    val visible = remember(hub.history, me, channel) {
        hub.history.filter { inChannel(it, channel, me) }.sortedByDescending { it.createdAtMs }
    }
    val uploads = hub.transfers.filter { it.upload && it.channel == channel }
    var selected by remember { mutableStateOf<Received?>(null) }
    val list = rememberLazyListState()
    LaunchedEffect(visible.firstOrNull()?.msgId, uploads.size) { list.scrollToItem(0) }
    LaunchedEffect(channel, visible.firstOrNull()?.msgId) {
        try {
            onOpened(channel)
        } catch (e: OtException) {
            Log.w("OldTown", "non-lus non rangés : ${e.message}")
        }
    }
    val swipe = with(LocalDensity.current) { SWIPE_BACK.toPx() }
    var dragged by remember { mutableFloatStateOf(0f) }

    Column(
        Modifier.fillMaxSize().pointerInput(onBack) {
            detectHorizontalDragGestures(
                onDragStart = { dragged = 0f },
                onDragEnd = { if (dragged > swipe) onBack() },
                onDragCancel = { dragged = 0f },
            ) { _, amount -> dragged += amount }
        },
    ) {
        Row(
            Modifier.fillMaxWidth().height(60.dp).padding(horizontal = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            GhostIcon(OtIcons.Back, "Retour", onBack)
            Spacer(Modifier.width(4.dp))
            Avatar(header.kind, header.color, size = 36.dp, initials = header.initials)
            Spacer(Modifier.width(10.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    header.name,
                    style = MaterialTheme.typography.bodyLarge.copy(fontWeight = FontWeight.SemiBold),
                    color = Ot.colors.foreground,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                val status = header.online?.let { it to if (it) "en ligne" else "hors ligne" }
                    ?: subtitle(presence, device, others.map { it.deviceId })
                status?.let { (online, text) ->
                    Text(
                        text,
                        style = MaterialTheme.typography.labelSmall,
                        color = if (online) Ot.colors.success else Ot.colors.muted,
                    )
                }
            }
        }

        LazyColumn(
            Modifier.weight(1f).fillMaxWidth().testTag("messages"),
            state = list,
            reverseLayout = true,
            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.Bottom),
        ) {
            items(uploads, key = { "up:" + it.id }) { UploadBubble(it) }
            items(visible, key = { it.msgId }) { item ->
                MessageBubble(
                    item,
                    sender = if (person != null) senderName(hub, item) else null,
                    progress = hub.transfers.firstOrNull { !it.upload && it.id == item.msgId && it.error == null },
                    onOpen = { selected = item },
                )
            }
            if (visible.isEmpty() && uploads.isEmpty()) {
                item {
                    Text(
                        "Aucun message avec ${header.name}.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = Ot.colors.muted,
                        modifier = Modifier.fillMaxWidth().padding(vertical = 32.dp),
                        textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                    )
                }
            }
        }

        when {
            header.readOnly -> Text(
                "Un service écrit sans lire les réponses.",
                style = MaterialTheme.typography.bodySmall,
                color = Ot.colors.muted,
                modifier = Modifier.fillMaxWidth().padding(16.dp),
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )
            else -> {
                if (header.outdated) {
                    Text(
                        "Cet appareil n'est pas à jour : il ne recevra rien.",
                        style = MaterialTheme.typography.bodySmall,
                        color = Ot.colors.warningForeground,
                        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 4.dp),
                    )
                }
                Composer(hub, channel, header.name)
            }
        }
    }
    selected?.let { ItemSheet(it, onDismiss = { selected = null }) }
}

private fun subtitle(presence: Map<String, Boolean>?, member: String?, others: List<String>): Pair<Boolean, String>? {
    if (presence == null) return null
    if (member != null) {
        val online = presence[member] == true
        return online to if (online) "en ligne" else "hors ligne"
    }
    val n = others.count { presence[it] == true }
    return (n > 0) to if (n > 0) "$n en ligne" else "aucun appareil en ligne"
}

private val BUBBLE = 16.dp
private val TAIL = 4.dp

@Composable
private fun MessageBubble(item: Received, sender: String?, progress: Transfer?, onOpen: () -> Unit) {
    val out = item.outgoing
    val c = Ot.colors
    Box(Modifier.fillMaxWidth(), contentAlignment = if (out) Alignment.CenterEnd else Alignment.CenterStart) {
        Column(
            Modifier.fillMaxWidth(0.86f),
            horizontalAlignment = if (out) Alignment.End else Alignment.Start,
        ) {
            if (sender != null && !out) {
                Text(
                    sender,
                    style = MaterialTheme.typography.labelSmall,
                    color = c.muted,
                    modifier = Modifier.padding(start = 4.dp, bottom = 2.dp),
                )
            }
            val shape = if (out) {
                RoundedCornerShape(BUBBLE, BUBBLE, TAIL, BUBBLE)
            } else {
                RoundedCornerShape(BUBBLE, BUBBLE, BUBBLE, TAIL)
            }
            val clickable = !(out && item.file != null)
            Column(
                Modifier
                    .clip(shape)
                    .background(if (out) c.bubbleOut else c.groupedCard)
                    .border(1.dp, if (out) c.bubbleOut else c.border, shape)
                    .then(if (clickable) Modifier.clickable(onClick = onOpen) else Modifier)
                    .padding(horizontal = 12.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(3.dp),
            ) {
                val fg = c.foreground
                val file = item.file
                if (file != null) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(OtIcons.Paperclip, null, tint = fg, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text(
                            file.name,
                            style = MaterialTheme.typography.bodyMedium,
                            color = fg,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    val state = when {
                        out -> ""
                        file.savedUri != null -> " · enregistré"
                        else -> ""
                    }
                    Text(
                        humanSize(file.size.toLong()) + state,
                        style = MaterialTheme.typography.bodySmall,
                        color = fg.copy(alpha = 0.75f),
                    )
                    if (progress != null) {
                        LinearProgressIndicator(
                            progress = { if (progress.total > 0) progress.done.toFloat() / progress.total else 0f },
                            modifier = Modifier.fillMaxWidth().height(3.dp),
                            color = c.primary,
                            trackColor = c.border,
                        )
                    }
                } else {
                    val link = item.kind == ContentKind.LINK
                    Text(
                        item.body,
                        style = MaterialTheme.typography.bodyMedium.copy(
                            textDecoration = if (link) TextDecoration.Underline else null,
                        ),
                        color = if (link) c.primaryText else fg,
                        maxLines = 12,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            Text(
                shortTime(item.createdAtMs) + if (item.pending) " · envoi…" else "",
                style = MaterialTheme.typography.labelSmall,
                color = c.muted,
                modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp),
            )
        }
    }
}

@Composable
private fun UploadBubble(t: Transfer) {
    val c = Ot.colors
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
        Column(
            Modifier
                .fillMaxWidth(0.86f)
                .padding(start = 0.dp),
            horizontalAlignment = Alignment.End,
        ) {
            Column(
                Modifier
                    .clip(RoundedCornerShape(BUBBLE, BUBBLE, TAIL, BUBBLE))
                    .background(c.bubbleOut)
                    .padding(horizontal = 12.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(OtIcons.Paperclip, null, tint = c.foreground, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(6.dp))
                    Text(t.name, style = MaterialTheme.typography.bodyMedium, color = c.foreground, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (t.error != null) {
                    Text(t.error, style = MaterialTheme.typography.bodySmall.copy(fontWeight = FontWeight.SemiBold), color = c.foreground)
                } else {
                    Text(
                        if (t.total > 0) "${t.done * 100 / t.total} %" else "…",
                        style = MaterialTheme.typography.bodySmall,
                        color = c.foreground.copy(alpha = 0.75f),
                    )
                    LinearProgressIndicator(
                        progress = { if (t.total > 0) t.done.toFloat() / t.total else 0f },
                        modifier = Modifier.fillMaxWidth().height(3.dp),
                        color = c.primary,
                        trackColor = c.border,
                    )
                }
            }
        }
    }
}

@Composable
private fun Composer(hub: HubState, channel: String, name: String) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var text by rememberSaveable(channel) { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var notice by remember { mutableStateOf<String?>(null) }
    val ready = hub.group != null
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        FileTransfers.keepAccess(context, uris)
        RelayHub.sendFiles(context, uris, channel)
    }

    Column(Modifier.fillMaxWidth().padding(start = 8.dp, end = 12.dp, top = 4.dp, bottom = 10.dp)) {
        val line = error ?: notice
        if (line != null) {
            Text(
                line,
                style = MaterialTheme.typography.bodySmall,
                color = if (error != null) Ot.colors.dangerForeground else Ot.colors.muted,
                modifier = Modifier.padding(start = 8.dp, bottom = 6.dp),
            )
        }
        Row(verticalAlignment = Alignment.Bottom) {
            GhostIcon(OtIcons.Paperclip, "Envoyer un fichier", { pickFiles.launch(arrayOf("*/*")) }, tint = Ot.colors.muted, enabled = ready)
            OtTextField(
                value = text,
                onValueChange = {
                    text = it
                    error = null
                },
                placeholder = "Message à $name",
                singleLine = false,
                maxLines = 5,
                modifier = Modifier.weight(1f).testTag("message-field"),
            )
            Spacer(Modifier.width(8.dp))
            val canSend = text.isNotBlank() && ready
            Box(
                Modifier
                    .size(40.dp)
                    .clip(CircleShape)
                    .background(if (canSend) Ot.colors.primary else Ot.colors.groupedCard)
                    .clickable(enabled = canSend) {
                        val body = text
                        text = ""
                        notice = if (hub.connection is Connection.Connected) null else "Hors ligne : envoi au retour de la connexion."
                        scope.launch {
                            val failure = try {
                                RelayHub.send(context, body, channel)
                                null
                            } catch (e: OtException) {
                                e.message ?: "envoi impossible"
                            } catch (_: TimeoutCancellationException) {
                                "le service ne répond pas"
                            }
                            notice = null
                            if (failure != null) {
                                error = failure
                                if (text.isEmpty()) text = body
                            }
                        }
                    }
                    .testTag("send"),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    OtIcons.Send,
                    contentDescription = "Envoyer",
                    tint = if (canSend) Ot.colors.onPrimary else Ot.colors.muted,
                    modifier = Modifier.size(18.dp),
                )
            }
        }
    }
}
