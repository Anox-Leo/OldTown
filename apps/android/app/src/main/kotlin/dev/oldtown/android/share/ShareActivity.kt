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
import dev.oldtown.android.ui.byUsage
import dev.oldtown.android.ui.deviceKind
import dev.oldtown.android.ui.components.Avatar
import dev.oldtown.android.ui.components.OtCard
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.theme.OldTownTheme
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.android.ui.theme.OtShapes
import dev.oldtown.core.OtException
import dev.oldtown.core.Received
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Cible du menu Partager (étape 3, T7 ; fichiers à l'étape 5) : feuille du
 * bas par-dessus l'appli d'origine, choix du destinataire, envoi par le
 * service, confirmation brève. Depuis un raccourci Direct Share, le
 * destinataire est déjà choisi. Les fichiers sont confiés au service, qui
 * les envoie après la fermeture de la feuille.
 */
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
        val direct = Shortcuts.memberOf(intent.getStringExtra(Intent.EXTRA_SHORTCUT_ID))
        setContent {
            OldTownTheme {
                ShareSheet(content, direct, onDone = { finish() })
            }
        }
    }
}

/** URI partagées : `EXTRA_STREAM` (une ou une liste), à défaut le `ClipData`. */
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

    fun send(to: List<String>, label: String) {
        if (content is SharedContent.Files) {
            // Le service garde l'accès aux URI et envoie après la fermeture.
            RelayHub.sendFiles(context, content.uris.map { it.toUri() }, to)
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
                RelayHub.send(context, text, to)
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

    // Direct Share : envoi immédiat au membre choisi.
    LaunchedEffect(hub.group, direct) {
        val group = hub.group ?: return@LaunchedEffect
        val member = group.members.firstOrNull { it.deviceId == direct } ?: return@LaunchedEffect
        if (state == SendState.Idle) {
            androidx.core.content.pm.ShortcutManagerCompat.reportShortcutUsed(context, Shortcuts.idOf(member.deviceId))
            send(listOf(member.deviceId), member.name)
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
                SendState.Idle -> Recipients(hub.group, hub.history, hub.device?.deviceId, onPick = ::send)
            }
        }
    }
}

/** Noms et tailles des fichiers partagés, lus auprès de leur fournisseur. */
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

/**
 * Destinataires : « Tous mes appareils » en tête, puis les appareils du plus
 * utilisé au moins utilisé (note 0033, [byUsage]), avec leur avatar.
 */
@Composable
private fun Recipients(
    group: dev.oldtown.core.Group?,
    history: List<Received>,
    me: String?,
    onPick: (List<String>, String) -> Unit,
) {
    if (group == null) {
        Banner("Groupe pas encore reçu : en attente du parrain.", BannerKind.Warning)
        return
    }
    val others = remember(group, history, me) { byUsage(group.members, history, me) }
    OtCard(padding = androidx.compose.foundation.layout.PaddingValues(0.dp)) {
        RecipientRow(
            ALL_NAME,
            "${others.size} appareil" + if (others.size > 1) "s" else "",
            DeviceKind.GROUP,
            avatarColor(ALL_CHANNEL),
            Modifier.testTag("recipient-all"),
        ) { onPick(emptyList(), "tous") }
        for (m in others) {
            Divider()
            RecipientRow(m.name, null, deviceKind(m.name), avatarColor(m.deviceId), Modifier.testTag("recipient")) {
                onPick(listOf(m.deviceId), m.name)
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
    onClick: () -> Unit,
) {
    Row(
        modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Avatar(kind, color, size = 36.dp)
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
