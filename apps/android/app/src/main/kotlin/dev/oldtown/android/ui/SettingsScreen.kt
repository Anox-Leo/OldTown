package dev.oldtown.android.ui

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
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import dev.oldtown.android.relay.Connection
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
import dev.oldtown.android.ui.components.OtTextField
import dev.oldtown.android.ui.components.SectionLabel
import dev.oldtown.android.ui.components.StatusPill
import dev.oldtown.android.ui.components.TopBar
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.android.ui.theme.OtShapes
import dev.oldtown.core.OtException
import dev.oldtown.core.clearHistory
import dev.oldtown.core.coreVersion
import dev.oldtown.core.forgetDevice
import dev.oldtown.core.setServerUrl
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Réglages et état (étape 3, T10). */
@Composable
fun SettingsScreen(hub: HubState, onBack: () -> Unit, onForgotten: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val device = hub.device ?: return
    var url by remember(device.serverUrl) { mutableStateOf(device.serverUrl) }
    var urlMessage by remember { mutableStateOf<Pair<Boolean, String>?>(null) }
    var forgetting by remember { mutableStateOf(false) }
    var notifications by remember { mutableStateOf(true) }
    var battery by remember { mutableStateOf(true) }
    LifecycleResumeEffect(Unit) {
        notifications = SystemState.notificationsAllowed(context)
        battery = SystemState.batteryExempt(context)
        onPauseOrDispose {}
    }
    val appVersion = remember {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "?"
    }

    Column(Modifier.fillMaxSize()) {
        TopBar("Réglages", onBack)
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SectionLabel("Connexion")
            OtCard {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("État", style = MaterialTheme.typography.bodyMedium, color = Ot.colors.foreground, modifier = Modifier.weight(1f))
                    val (label, color) = connectionLabel(hub)
                    StatusPill(label, color)
                }
                val conn = hub.connection
                if (conn is Connection.Reconnecting || conn is Connection.AuthFailed) {
                    Spacer(Modifier.height(10.dp))
                    Text(
                        if (conn is Connection.Reconnecting) conn.reason else (conn as Connection.AuthFailed).message,
                        style = MaterialTheme.typography.bodySmall,
                        color = Ot.colors.muted,
                    )
                    Spacer(Modifier.height(6.dp))
                    Text(
                        "Vérifier que Tailscale est connecté sur ce téléphone, et que l'URL ci-dessous est la bonne.",
                        style = MaterialTheme.typography.bodySmall,
                        color = Ot.colors.muted,
                    )
                }
                Spacer(Modifier.height(14.dp))
                Text("URL du relais", style = MaterialTheme.typography.labelMedium, color = Ot.colors.muted)
                Spacer(Modifier.height(6.dp))
                OtTextField(url, { url = it.trim(); urlMessage = null }, "wss://relais:7878", mono = true)
                Spacer(Modifier.height(10.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    urlMessage?.let { (ok, text) ->
                        Text(
                            text,
                            style = MaterialTheme.typography.bodySmall,
                            color = if (ok) Ot.colors.success else Ot.colors.dangerForeground,
                            modifier = Modifier.weight(1f),
                        )
                    } ?: Spacer(Modifier.weight(1f))
                    OtButton(
                        "Enregistrer",
                        {
                            scope.launch {
                                urlMessage = try {
                                    withContext(Dispatchers.IO) { setServerUrl(RelayHub.store(context), url) }
                                    RelayHub.reload(context)
                                    RelayHub.restart(context)
                                    true to "Enregistrée, reconnexion…"
                                } catch (e: OtException) {
                                    false to (e.message ?: "URL invalide")
                                }
                            }
                        },
                        kind = ButtonKind.Secondary,
                        enabled = url != device.serverUrl,
                    )
                }
                if (url.startsWith("ws://")) {
                    Spacer(Modifier.height(10.dp))
                    Banner("ws:// : connexion sans TLS, réservée aux essais.", BannerKind.Warning)
                }
            }

            SectionLabel("Cet appareil")
            OtCard(padding = PaddingValues(0.dp)) {
                ListRow("Nom", subtitle = "${device.name} · fixé à l'appairage")
                Divider()
                ListRow("Identifiant", subtitle = device.deviceId, subtitleMono = true)
            }

            SectionLabel("Téléphone")
            OtCard(padding = PaddingValues(0.dp)) {
                ListRow(
                    "Notifications",
                    subtitle = if (notifications) "Activées" else "Désactivées : rien ne s'affiche à la réception",
                    onClick = { SystemState.openNotificationSettings(context) },
                ) { Chevron() }
                Divider()
                ListRow(
                    "Batterie",
                    subtitle = if (battery) "Sans restriction : réception en veille" else "Optimisée : les messages peuvent attendre le réveil",
                    onClick = if (battery) null else ({ SystemState.requestBatteryExemption(context) }),
                ) { if (!battery) Chevron() }
            }

            SectionLabel("Historique")
            OtCard(padding = PaddingValues(0.dp)) {
                ListRow(
                    "Effacer les éléments reçus",
                    subtitle = "${hub.history.size} sur ce téléphone (50 au plus)",
                    leading = OtIcons.Trash,
                    onClick = {
                        scope.launch {
                            withContext(Dispatchers.IO) { clearHistory(RelayHub.store(context)) }
                            RelayHub.setHistory(emptyList())
                        }
                    },
                )
            }

            SectionLabel("Version")
            OtCard(padding = PaddingValues(0.dp)) {
                ListRow("Appli", subtitle = appVersion)
                Divider()
                ListRow("Cœur Rust", subtitle = coreVersion())
            }

            Spacer(Modifier.height(4.dp))
            OtButton(
                "Oublier cet appareil",
                { forgetting = true },
                Modifier.fillMaxWidth(),
                kind = ButtonKind.Danger,
                icon = OtIcons.Trash,
            )
            Spacer(Modifier.height(24.dp))
        }
    }

    if (forgetting) {
        AlertDialog(
            onDismissRequest = { forgetting = false },
            containerColor = Ot.colors.card,
            shape = OtShapes.card,
            title = { Text("Oublier cet appareil ?") },
            text = {
                Text(
                    "Clés, groupe et historique sont effacés de ce téléphone ; il faudra le réappairer. " +
                        "Depuis un ordinateur, l'exclure du groupe (ot group rotate --exclude ${device.name}) " +
                        "et le révoquer sur le relais.",
                    color = Ot.colors.muted,
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    forgetting = false
                    scope.launch {
                        RelayHub.stop(context)
                        withContext(Dispatchers.IO) { forgetDevice(RelayHub.store(context)) }
                        RelayHub.reload(context)
                        onForgotten()
                    }
                }) { Text("Oublier", color = Ot.colors.dangerForeground) }
            },
            dismissButton = {
                TextButton(onClick = { forgetting = false }) { Text("Annuler", color = Ot.colors.foreground) }
            },
        )
    }
}

@Composable
private fun Chevron() {
    androidx.compose.material3.Icon(
        OtIcons.ChevronRight,
        contentDescription = null,
        tint = Ot.colors.muted,
        modifier = Modifier.padding(start = 4.dp),
    )
}
