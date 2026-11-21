package dev.oldtown.android.ui.admin

import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.oldtown.android.notify.Clipboard
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.relay.Notice
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.attempt
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.components.ButtonKind
import dev.oldtown.android.ui.components.Divider
import dev.oldtown.android.ui.components.GhostIcon
import dev.oldtown.android.ui.components.ListRow
import dev.oldtown.android.ui.components.Mark
import dev.oldtown.android.ui.components.OtButton
import dev.oldtown.android.ui.components.OtCard
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.components.OtSwitch
import dev.oldtown.android.ui.components.OtTextField
import dev.oldtown.android.ui.components.SectionLabel
import dev.oldtown.android.ui.components.TopBar
import dev.oldtown.android.ui.pairing.QrImage
import dev.oldtown.android.ui.theme.Mono
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.core.PersonInvite
import java.text.DateFormat
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * Inviter une personne (T15, comme `admin.html`) : son nom, les personnes
 * avec qui elle sera liée (moi par défaut), puis le QR, le lien et le code
 * à lui transmettre à elle seule. Ce téléphone la certifie à l'arrivée de sa
 * demande (`PersonJoined`, avec l'empreinte à comparer de vive voix).
 */
@Composable
fun InviteScreen(hub: HubState, onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var name by rememberSaveable { mutableStateOf("") }
    val meUser = hub.me?.userId
    var links by remember(meUser) { mutableStateOf(setOfNotNull(meUser)) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var invite by remember { mutableStateOf<PersonInvite?>(null) }

    Column(Modifier.fillMaxSize()) {
        TopBar("Inviter une personne", onBack)
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (hub.rootMissing != null || hub.me?.isAdmin != true) {
                Banner("Réservé au téléphone de l'admin, une fois l'annuaire publié (réglages).", BannerKind.Warning)
                return@Column
            }
            val current = invite
            if (current != null) {
                InvitationCard(current, name.trim())
                OtButton("Terminé", onBack, Modifier.fillMaxWidth(), kind = ButtonKind.Secondary)
                return@Column
            }
            SectionLabel("Son nom")
            OtTextField(name, { name = it.take(64) }, "Bob", modifier = Modifier.testTag("invite-name"))
            Text(
                "Signé par ce téléphone : il s'affichera chez tous ceux qui lui sont liés.",
                style = MaterialTheme.typography.bodySmall,
                color = Ot.colors.muted,
            )
            val others = hub.people.filter { !it.isService }
            if (others.isNotEmpty()) {
                SectionLabel("Liée à")
                OtCard(padding = PaddingValues(0.dp)) {
                    others.forEachIndexed { i, p ->
                        if (i > 0) Divider()
                        ListRow(p.name + if (p.userId == meUser) " (moi)" else "") {
                            OtSwitch(
                                checked = p.userId in links,
                                onCheckedChange = { on -> links = if (on) links + p.userId else links - p.userId },
                            )
                        }
                    }
                }
            }
            error?.let { Banner(it, BannerKind.Danger) }
            OtButton(
                "Créer l'invitation",
                {
                    busy = true
                    error = null
                    scope.launch {
                        error = attempt { invite = RelayHub.client(context).invite(name.trim(), links.toList()) }
                        busy = false
                    }
                },
                Modifier.fillMaxWidth().testTag("invite-create"),
                enabled = name.isNotBlank(),
                loading = busy,
            )
            Spacer(Modifier.height(24.dp))
        }
    }
}

/**
 * Invitation créée (ou refaite) : QR à scanner, lien à envoyer (un SMS
 * ouvre l'appli), code ; puis l'arrivée de la personne et l'empreinte de son
 * appareil, à comparer de vive voix.
 */
@Composable
fun ColumnScope.InvitationCard(invite: PersonInvite, name: String) {
    val context = LocalContext.current
    var joined by remember(invite) { mutableStateOf<Notice.PersonJoined?>(null) }
    LaunchedEffect(invite) {
        joined = RelayHub.notices.filterIsInstance<Notice.PersonJoined>().first()
    }
    val arrived = joined
    if (arrived != null) {
        Banner(
            "${arrived.name} a rejoint OldTown depuis « ${arrived.deviceName} ». " +
                "Comparez de vive voix l'empreinte affichée sur son appareil : ${arrived.fingerprint}",
        )
        return
    }
    Mark(32.dp, Modifier.align(Alignment.CenterHorizontally))
    QrImage(invite.uri, Modifier.padding(horizontal = 24.dp).testTag("invite-qr"))
    OtCard {
        Text("Pour ${name.ifBlank { "la personne invitée" }}", style = MaterialTheme.typography.labelMedium, color = Ot.colors.foreground)
        Spacer(Modifier.height(6.dp))
        SelectionContainer {
            Text(invite.code, style = MaterialTheme.typography.bodySmall.copy(fontFamily = Mono), color = Ot.colors.foreground)
        }
        Spacer(Modifier.height(10.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                "Valable jusqu'au ${expiry(invite.expiresAtMs)}",
                style = MaterialTheme.typography.bodySmall,
                color = Ot.colors.muted,
                modifier = Modifier.weight(1f),
            )
            GhostIcon(OtIcons.Copy, "Copier le lien", {
                Clipboard.copy(context, invite.uri, sensitive = true)
            }, tint = Ot.colors.muted)
            GhostIcon(OtIcons.Share, "Envoyer le lien", {
                val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, invite.uri)
                context.startActivity(Intent.createChooser(send, null))
            }, tint = Ot.colors.muted)
        }
    }
    Banner(
        "Le QR, le lien et le code contiennent un secret : à elle seule. Ce téléphone l'accueillera dès sa " +
            "demande reçue, même plus tard : il doit rester dans OldTown.",
        BannerKind.Warning,
    )
    Text("En attente de son arrivée…", style = MaterialTheme.typography.bodySmall, color = Ot.colors.muted)
}

private fun expiry(ms: Long): String =
    DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT, Locale.FRENCH).format(Date(ms))
