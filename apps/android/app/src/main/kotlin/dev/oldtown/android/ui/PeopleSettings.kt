package dev.oldtown.android.ui

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.oldtown.android.relay.DirectoryAlert
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.components.ButtonKind
import dev.oldtown.android.ui.components.Divider
import dev.oldtown.android.ui.components.ListRow
import dev.oldtown.android.ui.components.OtButton
import dev.oldtown.android.ui.components.OtCard
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.components.OtSwitch
import dev.oldtown.android.ui.components.SectionLabel
import dev.oldtown.android.ui.theme.Mono
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.core.OtException
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.launch

/** Lance un appel au service ; renvoie le message d'erreur, `null` s'il a réussi. */
suspend fun attempt(block: suspend () -> Unit): String? = try {
    block()
    null
} catch (e: OtException) {
    e.message ?: "échec"
} catch (_: TimeoutCancellationException) {
    "le service ne répond pas"
}

/** Nom d'une personne de l'annuaire, son identifiant à défaut. */
fun nameOf(hub: HubState, user: String): String = hub.people.firstOrNull { it.userId == user }?.name ?: user

/**
 * Réglages de l'étape 10 (comme `people-settings.ts`) : publier l'annuaire
 * (D51), alertes de l'annuaire, mes appareils, services à couper, lien vers
 * l'administration. Rien sans annuaire, sauf « Publier l'annuaire ».
 */
@Composable
fun PeopleSettings(hub: HubState, open: (Screen) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }

    fun act(block: suspend () -> Unit) {
        busy = true
        error = null
        scope.launch {
            error = attempt(block)
            busy = false
        }
    }

    hub.rootMissing?.let { name ->
        SectionLabel("Annuaire")
        OtCard {
            Text(
                "Ce téléphone est à l'admin, mais l'annuaire n'est pas encore publié. Sa racine portera le nom " +
                    "« $name », signé une fois pour toutes : s'il ne convient pas, le changer d'abord sur le relais :",
                style = MaterialTheme.typography.bodySmall,
                color = Ot.colors.muted,
            )
            Spacer(Modifier.height(6.dp))
            Text(
                "ot-server user rename owner <nom>",
                style = MaterialTheme.typography.bodySmall.copy(fontFamily = Mono),
                color = Ot.colors.foreground,
            )
            Spacer(Modifier.height(12.dp))
            OtButton(
                "Publier l'annuaire",
                { act { RelayHub.publishDirectory(context) } },
                Modifier.fillMaxWidth().testTag("publish-directory"),
                loading = busy,
            )
        }
    }

    if (hub.alerts.isNotEmpty()) {
        SectionLabel("Alertes de l'annuaire")
        OtCard {
            for (a in hub.alerts) {
                Text(
                    when (a) {
                        DirectoryAlert.Fork ->
                            "Le relais publie pour vous un autre groupe que le vôtre : n'ajoutez ni ne retirez " +
                                "d'appareil avant d'avoir vérifié."
                        is DirectoryAlert.Rejected -> "Objet refusé pour ${nameOf(hub, a.userId)} : ${a.reason}."
                        is DirectoryAlert.KeysChanged ->
                            "Les clés de ${nameOf(hub, a.userId)} ont changé (nouvel appareil après une réinvitation ?) : " +
                                "à vérifier de vive voix."
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = Ot.colors.dangerForeground,
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(8.dp))
            }
            OtButton("Effacer les alertes", { RelayHub.dismissAlerts() }, kind = ButtonKind.Secondary)
        }
    }

    val me = hub.me ?: run {
        error?.let { Banner(it, BannerKind.Danger) }
        return
    }
    SectionLabel("Personnes")
    OtCard(padding = PaddingValues(0.dp)) {
        ListRow(
            "Mes appareils",
            subtitle = "Par défaut, joignables, catégorie, ligne de commande",
            leading = OtIcons.Phone,
            onClick = { open(Screen.MyDevices) },
            modifier = Modifier.testTag("my-devices"),
        ) { Icon(OtIcons.ChevronRight, null, tint = Ot.colors.muted) }
        if (me.isAdmin) {
            Divider()
            ListRow(
                "Administration",
                subtitle = "Personnes, invitations, liens, services",
                leading = OtIcons.Users,
                onClick = { open(Screen.Admin) },
                modifier = Modifier.testTag("admin"),
            ) { Icon(OtIcons.ChevronRight, null, tint = Ot.colors.muted) }
        }
    }

    val services = hub.people.filter { it.isService }
    if (services.isNotEmpty()) {
        SectionLabel("Services")
        OtCard(padding = PaddingValues(0.dp)) {
            services.forEachIndexed { i, svc ->
                if (i > 0) Divider()
                ListRow(svc.name, subtitle = if (svc.muted) "Coupé : il ne vous envoie plus rien" else "Reçu") {
                    OtSwitch(
                        checked = !svc.muted,
                        onCheckedChange = { on -> act { RelayHub.client(context).muteService(svc.userId, !on) } },
                        enabled = !busy,
                    )
                }
            }
        }
    }
    error?.let { Banner(it, BannerKind.Danger) }
}
