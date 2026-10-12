package dev.oldtown.android.ui

import android.text.format.DateFormat
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import dev.oldtown.android.notify.Clipboard
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.relay.Notice
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.components.ButtonKind
import dev.oldtown.android.ui.components.Divider
import dev.oldtown.android.ui.components.Dot
import dev.oldtown.android.ui.components.GhostIcon
import dev.oldtown.android.ui.components.ListRow
import dev.oldtown.android.ui.components.OtButton
import dev.oldtown.android.ui.components.OtCard
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.components.SectionLabel
import dev.oldtown.android.ui.components.TopBar
import dev.oldtown.android.ui.pairing.QrImage
import dev.oldtown.android.ui.pairing.QrScanner
import dev.oldtown.android.ui.theme.Mono
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.android.ui.theme.OtShapes
import dev.oldtown.core.Invitation
import dev.oldtown.core.Member
import dev.oldtown.core.OtException
import dev.oldtown.core.QrKind
import dev.oldtown.core.RelayDevice
import dev.oldtown.core.classifyQr
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/** Membres du groupe, empreintes, ajout et exclusion d'un appareil. */
@Composable
fun GroupScreen(hub: HubState, onBack: () -> Unit, open: (Screen) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var devices by remember { mutableStateOf<List<RelayDevice>?>(null) }
    var devicesError by remember { mutableStateOf<String?>(null) }
    var excluding by remember { mutableStateOf<Member?>(null) }
    var busy by remember { mutableStateOf(false) }
    var message by remember { mutableStateOf<Pair<BannerKind, String>?>(null) }

    suspend fun loadDevices() {
        devicesError = null
        devices = try {
            RelayHub.client(context).devices()
        } catch (e: OtException) {
            devicesError = e.message
            null
        } catch (_: TimeoutCancellationException) {
            devicesError = "le service ne répond pas"
            null
        }
    }
    LaunchedEffect(hub.group?.rosterVersion) { loadDevices() }

    Column(Modifier.fillMaxSize()) {
        TopBar("Groupe", onBack) {
            GhostIcon(OtIcons.Refresh, "Actualiser", { scope.launch { loadDevices() } })
        }
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            val group = hub.group
            if (group == null) {
                Banner("Groupe pas encore reçu : en attente du parrain.", BannerKind.Warning)
                return@Column
            }
            message?.let { (kind, text) -> Banner(text, kind) }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OtButton("Ajouter un appareil", { open(Screen.Sponsor) }, Modifier.weight(1f), icon = OtIcons.Scan)
                OtButton(
                    "Code",
                    { open(Screen.ShowCode) },
                    kind = ButtonKind.Secondary,
                    icon = OtIcons.QrCode,
                )
            }

            SectionLabel("Membres · époque ${group.epoch} · clé ${group.keyFingerprint}")
            OtCard(padding = PaddingValues(0.dp)) {
                group.members.forEachIndexed { i, m ->
                    if (i > 0) Divider()
                    val online = devices?.firstOrNull { it.deviceId == m.deviceId }?.online
                    ListRow(
                        title = if (m.isSelf) "${m.name}  (ce téléphone)" else m.name,
                        subtitle = m.signFingerprint,
                        subtitleMono = true,
                        leading = OtIcons.Laptop,
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            if (online != null && !m.isSelf) {
                                Dot(if (online) Ot.colors.success else Ot.colors.muted.copy(alpha = 0.4f))
                                Spacer(Modifier.width(4.dp))
                            }
                            if (!m.isSelf) {
                                GhostIcon(OtIcons.Trash, "Exclure ${m.name}", { excluding = m }, tint = Ot.colors.muted)
                            }
                        }
                    }
                }
            }
            Text(
                "Comparez ces empreintes avec « ot group show » sur un ordinateur : elles doivent être identiques.",
                style = MaterialTheme.typography.bodySmall,
                color = Ot.colors.muted,
                modifier = Modifier.padding(horizontal = 4.dp),
            )

            val strangers = devices?.filterNot { it.member }.orEmpty()
            if (strangers.isNotEmpty()) {
                SectionLabel("Inconnus du groupe")
                Banner(
                    "Enregistrés sur le relais mais absents du groupe : appairage inachevé, ou relais compromis. " +
                        "Ils ne lisent rien. À révoquer sur le relais s'ils vous sont inconnus : " +
                        strangers.joinToString { it.name },
                    BannerKind.Warning,
                )
            }
            devicesError?.let {
                Text(
                    "Présence des appareils indisponible : $it",
                    style = MaterialTheme.typography.bodySmall,
                    color = Ot.colors.muted,
                    modifier = Modifier.padding(horizontal = 4.dp),
                )
            }
            Spacer(Modifier.height(24.dp))
        }
    }

    excluding?.let { m ->
        AlertDialog(
            onDismissRequest = { if (!busy) excluding = null },
            containerColor = Ot.colors.card,
            shape = OtShapes.card,
            title = { Text("Exclure « ${m.name} » ?") },
            text = {
                Text(
                    "Une nouvelle clé de groupe est envoyée aux autres membres : « ${m.name} » ne lira plus " +
                        "les messages suivants. Pensez aussi à le révoquer sur le relais.",
                    color = Ot.colors.muted,
                )
            },
            confirmButton = {
                TextButton(enabled = !busy, onClick = {
                    busy = true
                    scope.launch {
                        message = try {
                            val epoch = RelayHub.client(context).rotate(listOf(m.deviceId))
                            BannerKind.Info to "« ${m.name} » exclu (époque $epoch). Sur le relais : " +
                                "ot-server device revoke ${m.name}"
                        } catch (e: OtException) {
                            BannerKind.Danger to (e.message ?: "rotation impossible")
                        } catch (_: TimeoutCancellationException) {
                            BannerKind.Danger to "le service ne répond pas"
                        }
                        busy = false
                        excluding = null
                    }
                }) { Text("Exclure", color = Ot.colors.dangerForeground) }
            },
            dismissButton = {
                TextButton(enabled = !busy, onClick = { excluding = null }) {
                    Text("Annuler", color = Ot.colors.foreground)
                }
            },
        )
    }
}

/**
 * Parrainage par QR inversé (note 0013) : on scanne le QR affiché par
 * `ot pair join --qr` sur le nouvel appareil, on confirme, et le téléphone
 * le fait enregistrer puis lui envoie le groupe.
 */
@Composable
fun SponsorScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var scanned by remember { mutableStateOf<Pair<String, String>?>(null) } // (qr, nom)
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var done by remember { mutableStateOf<String?>(null) }

    Column(Modifier.fillMaxSize()) {
        TopBar("Ajouter un appareil", onBack)
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            val name = done
            when {
                name != null -> {
                    Banner("« $name » a rejoint le groupe.")
                    OtButton("Terminé", onBack, Modifier.fillMaxWidth())
                }
                scanned != null -> {
                    val (qr, device) = scanned!!
                    OtCard {
                        Text("Ajouter « $device » au groupe ?", style = MaterialTheme.typography.titleSmall, color = Ot.colors.foreground)
                        Spacer(Modifier.height(6.dp))
                        Text(
                            "Il recevra la clé du groupe et pourra lire et envoyer comme vos autres appareils. " +
                                "Vérifiez que ce nom est bien celui affiché sur l'écran que vous venez de scanner.",
                            style = MaterialTheme.typography.bodySmall,
                            color = Ot.colors.muted,
                        )
                    }
                    error?.let { Banner(it, BannerKind.Danger) }
                    OtButton(
                        "Ajouter",
                        {
                            busy = true
                            error = null
                            scope.launch {
                                try {
                                    done = RelayHub.client(context).sponsor(qr).name
                                } catch (e: OtException) {
                                    error = e.message
                                } catch (_: TimeoutCancellationException) {
                                    error = "le service ne répond pas"
                                }
                                busy = false
                            }
                        },
                        Modifier.fillMaxWidth(),
                        loading = busy,
                    )
                    OtButton("Annuler", { scanned = null; error = null }, Modifier.fillMaxWidth(), kind = ButtonKind.Secondary, enabled = !busy)
                }
                else -> {
                    Text(
                        "Sur le nouvel appareil : ot pair join --qr --server <url> --name <nom>, puis visez le QR affiché.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = Ot.colors.muted,
                    )
                    error?.let { Banner(it, BannerKind.Danger) }
                    QrScanner(
                        onResult = { text ->
                            when (val kind = classifyQr(text)) {
                                is QrKind.Sponsor -> {
                                    error = null
                                    scanned = text to kind.name
                                }
                                is QrKind.Join -> error = "Ce QR sert à rejoindre un groupe, pas à y ajouter un appareil."
                                is QrKind.Invalid -> error = kind.reason
                                QrKind.Unknown -> error = "Ce n'est pas un QR OldTown."
                            }
                        },
                        modifier = Modifier.fillMaxWidth().aspectRatio(1f),
                    )
                    if (error != null) {
                        OtButton("Scanner à nouveau", { error = null; scanned = null }, Modifier.fillMaxWidth(), kind = ButtonKind.Secondary)
                    }
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

/**
 * Code d'appairage créé par le téléphone, en QR et en texte, pour un appareil
 * qui rejoint avec `ot pair join` (ou un autre téléphone).
 */
@Composable
fun ShowCodeScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    var invitation by remember { mutableStateOf<Invitation?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var joined by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(Unit) {
        try {
            invitation = RelayHub.client(context).pairNew()
        } catch (e: OtException) {
            error = e.message
        } catch (_: TimeoutCancellationException) {
            error = "le service ne répond pas"
        }
    }
    LaunchedEffect(invitation) {
        if (invitation == null) return@LaunchedEffect
        joined = RelayHub.notices.filterIsInstance<Notice.PairingCompleted>().first().name
    }

    Column(Modifier.fillMaxSize()) {
        TopBar("Code d'appairage", onBack)
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            error?.let { Banner(it, BannerKind.Danger) }
            joined?.let {
                Banner("« $it » a rejoint le groupe.")
                OtButton("Terminé", onBack, Modifier.fillMaxWidth())
            }
            val inv = invitation
            if (inv != null && joined == null) {
                QrImage(inv.uri, Modifier.padding(horizontal = 24.dp))
                OtCard {
                    SelectionContainer {
                        Text(inv.code, style = MaterialTheme.typography.bodySmall.copy(fontFamily = Mono), color = Ot.colors.foreground)
                    }
                    Spacer(Modifier.height(10.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            "Valable jusqu'à ${DateFormat.getTimeFormat(context).format(inv.expiresAtMs)}, une seule fois",
                            style = MaterialTheme.typography.bodySmall,
                            color = Ot.colors.muted,
                            modifier = Modifier.weight(1f),
                        )
                        GhostIcon(OtIcons.Copy, "Copier le code", {
                            Clipboard.copy(context, inv.code, sensitive = true)
                        }, tint = Ot.colors.muted)
                    }
                }
                Banner(
                    "Le code contient un secret : ne le montrez qu'à l'appareil qui rejoint, sans passer par un service tiers. " +
                        "Gardez cet écran ouvert jusqu'à la fin.",
                    BannerKind.Warning,
                )
                Text(
                    "Sur l'ordinateur : ot pair join --server <url> --name <nom> <code>",
                    style = MaterialTheme.typography.bodySmall.copy(fontFamily = Mono),
                    color = Ot.colors.muted,
                )
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}
