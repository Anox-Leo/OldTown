package dev.oldtown.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.components.Avatar
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.components.OtCard
import dev.oldtown.android.ui.components.OtSwitch
import dev.oldtown.android.ui.components.SectionLabel
import dev.oldtown.android.ui.components.TopBar
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.android.ui.theme.OtShapes
import dev.oldtown.core.PersonDevice
import kotlinx.coroutines.launch

private val KINDS = listOf(
    "phone" to "Téléphone",
    "laptop" to "Portable",
    "desktop" to "Ordinateur fixe",
    "browser" to "Navigateur",
    "cli" to "Ligne de commande",
)

private fun kindLabel(kind: String?): String = KINDS.firstOrNull { it.first == kind }?.second ?: "Non précisée"

@Composable
fun MyDevicesScreen(hub: HubState, onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var removing by remember { mutableStateOf<PersonDevice?>(null) }
    var showCli by remember { mutableStateOf(false) }
    val me = hub.device?.deviceId
    val devices = hub.me?.devices.orEmpty()
    val (cli, usual) = devices.partition { deviceKind(it.kind) == DeviceKind.CLI }

    fun act(block: suspend () -> Unit) {
        busy = true
        error = null
        scope.launch {
            error = attempt(block)
            busy = false
        }
    }

    fun update(d: PersonDevice, kind: String? = null, reachable: Boolean? = null, default: Boolean? = null) =
        act { RelayHub.client(context).updateDevice(d.deviceId, kind, reachable, default) }

    Column(Modifier.fillMaxSize()) {
        TopBar("Mes appareils", onBack)
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (hub.me == null) {
                Banner("Pas d'annuaire : ce téléphone ne partage OldTown avec personne d'autre.", BannerKind.Info)
                return@Column
            }
            Text(
                "Une autre personne qui vous écrit vise vos appareils par défaut ; elle peut choisir un de vos appareils joignables.",
                style = MaterialTheme.typography.bodySmall,
                color = Ot.colors.muted,
            )
            error?.let { Banner(it, BannerKind.Danger) }
            for (d in usual) {
                DeviceCard(d, self = d.deviceId == me, busy = busy, onKind = { k ->
                    if (k == "cli") update(d, kind = k, reachable = false, default = false) else update(d, kind = k)
                }, onDefault = { update(d, default = it) }, onReachable = { update(d, reachable = it) }, onRemove = { removing = d })
            }
            if (cli.isNotEmpty()) {
                Text(
                    (if (showCli) "▾ " else "▸ ") + "Ligne de commande (${cli.size})",
                    style = MaterialTheme.typography.labelLarge,
                    color = Ot.colors.foreground,
                    modifier = Modifier.fillMaxWidth().clickable { showCli = !showCli }.padding(vertical = 8.dp).testTag("cli-devices"),
                )
                if (showCli) {
                    Text(
                        "Membres du groupe, jamais joignables ni par défaut.",
                        style = MaterialTheme.typography.bodySmall,
                        color = Ot.colors.muted,
                    )
                    for (d in cli) {
                        DeviceCard(d, self = d.deviceId == me, busy = busy, onKind = { k -> update(d, kind = k) },
                            onDefault = null, onReachable = null, onRemove = { removing = d })
                    }
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }

    removing?.let { d ->
        AlertDialog(
            onDismissRequest = { if (!busy) removing = null },
            containerColor = Ot.colors.card,
            shape = OtShapes.card,
            title = { Text("Retirer « ${d.name} » ?") },
            text = {
                Text(
                    "Nouvelle clé de groupe sans lui, puis révocation par le relais : il ne lira plus rien.",
                    color = Ot.colors.muted,
                )
            },
            confirmButton = {
                TextButton(enabled = !busy, onClick = {
                    removing = null
                    act { RelayHub.client(context).removeDevice(d.deviceId) }
                }) { Text("Retirer", color = Ot.colors.dangerForeground) }
            },
            dismissButton = {
                TextButton(enabled = !busy, onClick = { removing = null }) { Text("Annuler", color = Ot.colors.foreground) }
            },
        )
    }
}

@Composable
private fun DeviceCard(
    d: PersonDevice,
    self: Boolean,
    busy: Boolean,
    onKind: (String) -> Unit,
    onDefault: ((Boolean) -> Unit)?,
    onReachable: ((Boolean) -> Unit)?,
    onRemove: () -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    OtCard(Modifier.testTag("my-device")) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Avatar(deviceKind(d.kind), avatarColor(d.deviceId), size = 36.dp, online = d.online)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    d.name,
                    style = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.SemiBold),
                    color = Ot.colors.foreground,
                )
                Text(
                    when {
                        self -> "ce téléphone"
                        d.online -> "en ligne"
                        else -> "hors ligne"
                    },
                    style = MaterialTheme.typography.labelSmall,
                    color = if (d.online && !self) Ot.colors.success else Ot.colors.muted,
                )
            }
            if (!self) {
                TextButton(onClick = onRemove, enabled = !busy) { Text("Retirer…", color = Ot.colors.dangerForeground) }
            }
        }
        Spacer(Modifier.height(10.dp))
        Box {
            Text(
                "Catégorie : ${kindLabel(d.kind)}",
                style = MaterialTheme.typography.bodySmall,
                color = Ot.colors.primaryText,
                modifier = Modifier.clickable(enabled = !busy) { menu = true }.padding(vertical = 6.dp),
            )
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                for ((value, label) in KINDS) {
                    DropdownMenuItem(
                        text = { Text(label) },
                        onClick = {
                            menu = false
                            if (value != d.kind) onKind(value)
                        },
                    )
                }
            }
        }
        if (onDefault != null) {
            Toggle("Par défaut", d.default, busy, onDefault)
        }
        if (onReachable != null) {
            Toggle("Joignable", d.reachable, busy, onReachable)
        }
        if (!d.upToDate) {
            Text(
                "Pas à jour : il ne recevra rien des autres personnes.",
                style = MaterialTheme.typography.bodySmall,
                color = Ot.colors.warningForeground,
            )
        }
    }
}

@Composable
private fun Toggle(label: String, checked: Boolean, busy: Boolean, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, style = MaterialTheme.typography.bodySmall, color = Ot.colors.foreground, modifier = Modifier.weight(1f))
        OtSwitch(checked, onChange, enabled = !busy)
    }
}
