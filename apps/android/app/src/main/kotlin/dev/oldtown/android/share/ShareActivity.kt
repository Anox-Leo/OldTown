package dev.oldtown.android.share

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.IntentCompat
import androidx.core.net.toUri
import dev.oldtown.android.files.SharedFile
import dev.oldtown.android.files.describe
import dev.oldtown.android.files.humanSize
import dev.oldtown.android.notify.isWebLink
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.components.Divider
import dev.oldtown.android.ui.ALL_CHANNEL
import dev.oldtown.android.ui.ALL_NAME
import dev.oldtown.android.ui.DeviceKind
import dev.oldtown.android.ui.avatarColor
import dev.oldtown.android.ui.initials
import dev.oldtown.android.ui.shareTargets
import dev.oldtown.android.ui.components.Avatar
import dev.oldtown.android.ui.components.OtCard
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.theme.OldTownTheme
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.android.ui.theme.OtShapes
import dev.oldtown.core.OtException
import dev.oldtown.core.PersonItem
import dev.oldtown.core.Received
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class ShareActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val content = readShare(
            intent.action,
            intent.type,
            intent.getCharSequenceExtra(Intent.EXTRA_TEXT),
            streams(intent),
        )
        val direct = Shortcuts.targetOf(intent.getStringExtra(Intent.EXTRA_SHORTCUT_ID))
        setContent {
            OldTownTheme {
                ShareSheet(content, direct, onDone = { finish() })
            }
        }
    }
}

private fun streams(intent: Intent): List<String> {
    val list = if (intent.action == Intent.ACTION_SEND_MULTIPLE) {
        IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, android.net.Uri::class.java).orEmpty()
    } else {
        listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, android.net.Uri::class.java))
    }
    val clip = intent.clipData
    val fromClip = (0 until (clip?.itemCount ?: 0)).mapNotNull { clip?.getItemAt(it)?.uri }
    return (list.ifEmpty { fromClip }).map { it.toString() }
}

private sealed interface SendState {
    data object Idle : SendState
    data class Sending(val to: String) : SendState
    data class Sent(val to: String) : SendState
    data class Failed(val message: String) : SendState
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ShareSheet(content: SharedContent, direct: String?, onDone: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val hub by RelayHub.state.collectAsState()
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    var state by remember { mutableStateOf<SendState>(SendState.Idle) }

    LaunchedEffect(Unit) {
        RelayHub.reload(context)
        RelayHub.ensureStarted(context)
    }

    fun send(channel: String, label: String) {
        if (content is SharedContent.Files) {
            RelayHub.sendFiles(context, content.uris.map { it.toUri() }, channel)
            state = SendState.Sent(label)
            scope.launch {
                delay(900)
                onDone()
            }
            return
        }
        val text = (content as? SharedContent.Text)?.text ?: return
        state = SendState.Sending(label)
        scope.launch {
            state = try {
                RelayHub.send(context, text, channel)
                SendState.Sent(label)
            } catch (e: OtException) {
                SendState.Failed(e.message ?: "envoi impossible")
            } catch (e: kotlinx.coroutines.TimeoutCancellationException) {
                SendState.Failed("le service ne répond pas")
            }
            if (state is SendState.Sent) {
                delay(700)
                onDone()
            }
        }
    }

    LaunchedEffect(hub.group, hub.people, direct) {
        val group = hub.group ?: return@LaunchedEffect
        if (direct == null) return@LaunchedEffect
        val target = shareTargets(group.members, hub.history, hub.device?.deviceId, hub.people)
            .firstOrNull { it.channel == direct } ?: return@LaunchedEffect
        if (state == SendState.Idle) {
            androidx.core.content.pm.ShortcutManagerCompat.reportShortcutUsed(context, Shortcuts.idOf(target.channel))
            send(target.channel, target.name)
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDone,
        sheetState = sheet,
        shape = OtShapes.sheet,
        containerColor = Ot.colors.card,
        scrimColor = Ot.colors.backdrop,
        dragHandle = null,
    ) {
        Column(
            Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .padding(horizontal = 16.dp, vertical = 18.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Envoyer avec OldTown", style = MaterialTheme.typography.titleMedium, color = Ot.colors.foreground)
            when (content) {
                is SharedContent.Files -> FilesPreview(content.uris)
                SharedContent.Empty -> {
                    Banner("Rien à envoyer.")
                    return@Column
                }
                is SharedContent.Text -> Preview(content.text)
            }
            if (hub.loaded && hub.device == null) {
                Banner("Ce téléphone n'est pas encore appairé : ouvrir OldTown pour rejoindre un groupe.", BannerKind.Warning)
                return@Column
            }
            when (val s = state) {
                is SendState.Sending -> Status("Envoi à ${s.to}…", done = false)
                is SendState.Sent -> Status(
                    if (content is SharedContent.Files) "Envoi à ${s.to} lancé" else "Envoyé à ${s.to}",
                    done = true,
                )
                is SendState.Failed -> Banner(s.message, BannerKind.Danger, "Réessayer", { state = SendState.Idle })
                SendState.Idle -> Recipients(hub.group, hub.history, hub.device?.deviceId, hub.people, onPick = ::send)
            }
        }
    }
}

@Composable
private fun FilesPreview(uris: List<String>) {
    val context = androidx.compose.ui.platform.LocalContext.current
    var files by remember { mutableStateOf<List<SharedFile>?>(null) }
    LaunchedEffect(uris) {
        files = withContext(Dispatchers.IO) {
            uris.mapNotNull { runCatching { describe(context, it.toUri()) }.getOrNull() }
        }
    }
    val list = files ?: return
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        for (f in list.take(MAX_PREVIEW)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(OtIcons.Paperclip, null, tint = Ot.colors.muted, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(10.dp))
                Text(
                    f.name,
                    style = MaterialTheme.typography.bodySmall,
                    color = Ot.colors.foreground,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                if (f.size >= 0) {
                    Spacer(Modifier.width(8.dp))
                    Text(humanSize(f.size), style = MaterialTheme.typography.bodySmall, color = Ot.colors.muted)
                }
            }
        }
        if (list.size > MAX_PREVIEW) {
            Text("et ${list.size - MAX_PREVIEW} autre(s)", style = MaterialTheme.typography.bodySmall, color = Ot.colors.muted)
        }
    }
}

private const val MAX_PREVIEW = 5

@Composable
private fun Preview(text: String) {
    val link = isWebLink(text)
    Row(
        Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(
            if (link) OtIcons.Link else OtIcons.Copy,
            contentDescription = null,
            tint = Ot.colors.muted,
            modifier = Modifier.padding(top = 2.dp).size(16.dp),
        )
        Spacer(Modifier.width(10.dp))
        Text(
            text.trim(),
            style = MaterialTheme.typography.bodySmall,
            color = if (link) Ot.colors.primaryText else Ot.colors.muted,
            maxLines = 3,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun Recipients(
    group: dev.oldtown.core.Group?,
    history: List<Received>,
    me: String?,
    people: List<PersonItem>,
    onPick: (String, String) -> Unit,
) {
    if (group == null) {
        Banner("Groupe pas encore reçu : en attente du parrain.", BannerKind.Warning)
        return
    }
    val targets = remember(group, history, me, people) { shareTargets(group.members, history, me, people) }
    val (others, persons) = targets.partition { it.kind != DeviceKind.PERSON }
    OtCard(padding = androidx.compose.foundation.layout.PaddingValues(0.dp)) {
        RecipientRow(
            ALL_NAME,
            "${others.size} appareil" + if (others.size > 1) "s" else "",
            DeviceKind.GROUP,
            avatarColor(ALL_CHANNEL),
            Modifier.testTag("recipient-all"),
        ) { onPick(ALL_CHANNEL, "tous") }
        for (t in others) {
            Divider()
            RecipientRow(t.name, null, t.kind, t.color, Modifier.testTag("recipient")) { onPick(t.channel, t.name) }
        }
    }
    if (persons.isNotEmpty()) {
        Text("Personnes", style = MaterialTheme.typography.labelMedium, color = Ot.colors.muted)
        OtCard(padding = androidx.compose.foundation.layout.PaddingValues(0.dp)) {
            persons.forEachIndexed { i, t ->
                if (i > 0) Divider()
                RecipientRow(
                    t.name,
                    "ses appareils par défaut",
                    t.kind,
                    t.color,
                    Modifier.testTag("recipient-person"),
                    initials = initials(t.name),
                ) { onPick(t.channel, t.name) }
            }
        }
    }
}

@Composable
private fun RecipientRow(
    name: String,
    subtitle: String?,
    kind: DeviceKind,
    color: Long,
    modifier: Modifier = Modifier,
    initials: String = "",
    onClick: () -> Unit,
) {
    Row(
        modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Avatar(kind, color, size = 36.dp, initials = initials)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(name, style = MaterialTheme.typography.bodyMedium, color = Ot.colors.foreground)
            if (subtitle != null) {
                Text(subtitle, style = MaterialTheme.typography.bodySmall, color = Ot.colors.muted)
            }
        }
        Icon(OtIcons.ChevronRight, null, tint = Ot.colors.muted, modifier = Modifier.size(18.dp))
    }
}

@Composable
private fun Status(text: String, done: Boolean) {
    Row(
        Modifier.fillMaxWidth().height(48.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.Center,
    ) {
        if (done) {
            Icon(OtIcons.Check, null, tint = Ot.colors.success, modifier = Modifier.size(20.dp))
        } else {
            androidx.compose.material3.CircularProgressIndicator(
                Modifier.size(18.dp),
                color = Ot.colors.primary,
                strokeWidth = 2.dp,
            )
        }
        Spacer(Modifier.width(10.dp))
        Text(text, style = MaterialTheme.typography.bodyMedium, color = Ot.colors.foreground)
    }
}
