package dev.oldtown.android.ui.admin

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.Screen
import dev.oldtown.android.ui.attempt
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.components.ButtonKind
import dev.oldtown.android.ui.components.Divider
import dev.oldtown.android.ui.components.GhostIcon
import dev.oldtown.android.ui.components.ListRow
import dev.oldtown.android.ui.components.OtButton
import dev.oldtown.android.ui.components.OtCard
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.components.OtSwitch
import dev.oldtown.android.ui.components.OtTextField
import dev.oldtown.android.ui.components.SectionLabel
import dev.oldtown.android.ui.components.TopBar
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.android.ui.theme.OtShapes
import dev.oldtown.core.AdminUserItem
import dev.oldtown.core.PersonInvite
import kotlinx.coroutines.launch

/** Personnes telles que le relais les connaît (admin), relues à chaque changement de l'annuaire. */
@Composable
private fun rememberUsers(hub: HubState): Triple<List<AdminUserItem>?, String?, suspend () -> Unit> {
    val context = LocalContext.current
    var users by remember { mutableStateOf<List<AdminUserItem>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val load: suspend () -> Unit = {
        error = attempt { users = RelayHub.client(context).users() }
    }
    LaunchedEffect(hub.people) { load() }
    return Triple(users, error, load)
}

private fun stateLabel(u: AdminUserItem): String = when (u.state) {
    "pending" -> "invitée, pas encore arrivée"
    "revoked" -> "révoquée"
    else -> {
        val n = u.devices.size
        val online = u.devices.count { it.online }
        "$n appareil" + (if (n > 1) "s" else "") + if (online > 0) " · $online en ligne" else ""
    }
}

/**
 * Administration (T15, comme `admin.html`) : inviter, ajouter un appareil
 * de service, puis les personnes et les services ; une ligne ouvre sa fiche
 * (liens, renommer, réinviter, révoquer).
 */
@Composable
fun AdminScreen(hub: HubState, onBack: () -> Unit, open: (Screen) -> Unit) {
    val scope = rememberCoroutineScope()
    val (users, error, load) = rememberUsers(hub)

    Column(Modifier.fillMaxSize()) {
        TopBar("Administration", onBack) {
            GhostIcon(OtIcons.Refresh, "Actualiser", { scope.launch { load() } })
        }
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (hub.rootMissing != null) {
                Banner("Annuaire pas encore publié : le publier d'abord, dans les réglages.", BannerKind.Warning)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OtButton("Inviter", { open(Screen.Invite) }, Modifier.weight(1f).testTag("admin-invite"), icon = OtIcons.Plus)
                OtButton("Service", { open(Screen.AddService) }, kind = ButtonKind.Secondary, icon = OtIcons.Scan)
            }
            error?.let { Banner(it, BannerKind.Danger) }
            val list = users ?: return@Column
            val (services, people) = list.partition { it.role == "service" }
            val (gone, present) = people.partition { it.state == "revoked" }
            UserList("Personnes", present, open)
            UserList("Services", services.filter { it.present() }, open)
            UserList("Révoquées", gone, open)
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun UserList(title: String, users: List<AdminUserItem>, open: (Screen) -> Unit) {
    if (users.isEmpty()) return
    SectionLabel(title)
    OtCard(padding = PaddingValues(0.dp)) {
        users.forEachIndexed { i, u ->
            if (i > 0) Divider()
            ListRow(
                u.name + if (u.role == "admin") "  (admin)" else "",
                subtitle = stateLabel(u),
                onClick = { open(Screen.Person(u.userId)) },
                modifier = Modifier.testTag("admin-user"),
            ) { Icon(OtIcons.ChevronRight, null, tint = Ot.colors.muted) }
        }
    }
}

private enum class Confirm { Revoke, Rename }

/**
 * Fiche d'une personne (admin) : ses appareils, ses liens en interrupteurs
 * (dans les deux sens entre personnes ; d'un service vers une personne, à
 * sens unique, D41), renommer, réinviter (appareils perdus), révoquer.
 */
@Composable
fun PersonAdminScreen(hub: HubState, userId: String, onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val (users, loadError, load) = rememberUsers(hub)
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var confirm by remember { mutableStateOf<Confirm?>(null) }
    var newName by remember { mutableStateOf("") }
    var reinvited by remember { mutableStateOf<PersonInvite?>(null) }
    val u = users?.firstOrNull { it.userId == userId }
    val isMe = userId == hub.me?.userId

    fun act(block: suspend () -> Unit) {
        busy = true
        error = null
        scope.launch {
            error = attempt(block) ?: attempt { load() }
            busy = false
        }
    }

    Column(Modifier.fillMaxSize()) {
        TopBar(u?.name ?: "Personne", onBack)
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            (error ?: loadError)?.let { Banner(it, BannerKind.Danger) }
            if (u == null) return@Column
            val all = users.orEmpty()
            val service = u.role == "service"
            Text(stateLabel(u), style = MaterialTheme.typography.bodySmall, color = Ot.colors.muted)

            reinvited?.let { inv ->
                InvitationCard(inv, u.name)
                OtButton("Terminé", { reinvited = null }, Modifier.fillMaxWidth(), kind = ButtonKind.Secondary)
            }

            if (u.devices.isNotEmpty()) {
                SectionLabel("Appareils")
                OtCard(padding = PaddingValues(0.dp)) {
                    u.devices.forEachIndexed { i, d ->
                        if (i > 0) Divider()
                        val notes = listOfNotNull(
                            if (d.online) "en ligne" else "hors ligne",
                            "par défaut".takeIf { d.default },
                            "pas joignable".takeIf { !d.reachable },
                            "pas à jour".takeIf { !d.upToDate },
                        )
                        ListRow(d.name, subtitle = notes.joinToString(" · "))
                    }
                }
            }

            if (u.present()) {
                SectionLabel(if (service) "Écrit à" else "Liens")
                OtCard(padding = PaddingValues(0.dp)) {
                    val candidates = linkCandidates(all, u.userId)
                    if (candidates.isEmpty()) ListRow("Personne d'autre pour l'instant")
                    candidates.forEachIndexed { i, other ->
                        if (i > 0) Divider()
                        if (service) {
                            ListRow(other.name, subtitle = "reçoit ses messages") {
                                OtSwitch(
                                    checked = other.userId in u.links,
                                    onCheckedChange = { on -> act { RelayHub.client(context).linkSet(u.userId, other.userId, on) } },
                                    enabled = !busy,
                                )
                            }
                        } else {
                            val link = linkBetween(all, u.userId, other.userId)
                            ListRow(
                                other.name,
                                subtitle = when (link) {
                                    Link.BOTH -> "s'écrivent"
                                    Link.ONE_WAY -> "à sens unique : réactiver pour les deux sens"
                                    Link.NONE -> "pas de lien"
                                },
                            ) {
                                OtSwitch(
                                    checked = link == Link.BOTH,
                                    onCheckedChange = { on ->
                                        act {
                                            val c = RelayHub.client(context)
                                            c.linkSet(u.userId, other.userId, on)
                                            c.linkSet(other.userId, u.userId, on)
                                        }
                                    },
                                    enabled = !busy,
                                )
                            }
                        }
                    }
                }
                val services = if (service) emptyList() else servicesFor(all, u.userId)
                if (services.isNotEmpty()) {
                    SectionLabel("Services qui lui écrivent")
                    OtCard(padding = PaddingValues(0.dp)) {
                        services.forEachIndexed { i, svc ->
                            if (i > 0) Divider()
                            ListRow(svc.name, subtitle = if (svc.userId in u.muted) "coupé par elle" else null) {
                                OtSwitch(
                                    checked = u.userId in svc.links,
                                    onCheckedChange = { on -> act { RelayHub.client(context).linkSet(svc.userId, u.userId, on) } },
                                    enabled = !busy,
                                )
                            }
                        }
                    }
                }
            }

            SectionLabel("Gérer")
            OtCard(padding = PaddingValues(0.dp)) {
                ListRow("Renommer", subtitle = "Son nom est re-signé pour tous", onClick = {
                    newName = u.name
                    confirm = Confirm.Rename
                })
                if (!isMe && !service && u.present()) {
                    Divider()
                    ListRow("Réinviter", subtitle = "Appareils perdus : nouvelle invitation, nouvelles clés", onClick = {
                        act { reinvited = RelayHub.client(context).reinvite(u.userId) }
                    })
                }
                if (!isMe && u.present()) {
                    Divider()
                    ListRow("Révoquer", subtitle = "Ses appareils ne recevront ni n'enverront plus rien", onClick = {
                        confirm = Confirm.Revoke
                    })
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }

    when (confirm) {
        Confirm.Revoke -> AlertDialog(
            onDismissRequest = { confirm = null },
            containerColor = Ot.colors.card,
            shape = OtShapes.card,
            title = { Text("Révoquer « ${u?.name} » ?") },
            text = { Text("Le relais refuse désormais tous ses appareils. Une nouvelle invitation reste possible.", color = Ot.colors.muted) },
            confirmButton = {
                TextButton(onClick = {
                    confirm = null
                    act { RelayHub.client(context).revokeUser(userId) }
                }) { Text("Révoquer", color = Ot.colors.dangerForeground) }
            },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text("Annuler", color = Ot.colors.foreground) } },
        )
        Confirm.Rename -> AlertDialog(
            onDismissRequest = { confirm = null },
            containerColor = Ot.colors.card,
            shape = OtShapes.card,
            title = { Text("Renommer") },
            text = { OtTextField(newName, { newName = it.take(64) }, "Nom") },
            confirmButton = {
                TextButton(enabled = newName.isNotBlank(), onClick = {
                    confirm = null
                    val name = newName.trim()
                    act { RelayHub.client(context).renameUser(userId, name) }
                }) { Text("Renommer", color = Ot.colors.primaryText) }
            },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text("Annuler", color = Ot.colors.foreground) } },
        )
        null -> Unit
    }
}
