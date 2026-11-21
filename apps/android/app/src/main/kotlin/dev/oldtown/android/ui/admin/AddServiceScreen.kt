package dev.oldtown.android.ui.admin

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.attempt
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.components.ButtonKind
import dev.oldtown.android.ui.components.OtButton
import dev.oldtown.android.ui.components.OtCard
import dev.oldtown.android.ui.components.OtTextField
import dev.oldtown.android.ui.components.SectionLabel
import dev.oldtown.android.ui.components.TopBar
import dev.oldtown.android.ui.pairing.QrScanner
import dev.oldtown.android.ui.theme.Mono
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.core.QrKind
import dev.oldtown.core.ServiceAdded
import dev.oldtown.core.classifyQr
import kotlinx.coroutines.launch

@Composable
fun AddServiceScreen(hub: HubState, onBack: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var service by rememberSaveable { mutableStateOf("") }
    var scanned by remember { mutableStateOf<Pair<String, String>?>(null) }
    var scanning by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var added by remember { mutableStateOf<ServiceAdded?>(null) }
    val existing = hub.people.filter { it.isService }

    Column(Modifier.fillMaxSize()) {
        TopBar("Appareil de service", onBack)
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            error?.let { Banner(it, BannerKind.Danger) }
            val done = added
            val pending = scanned
            when {
                done != null -> {
                    Banner(
                        "« ${done.deviceName} » ajouté au service " + (if (done.created) "créé : " else "") + "${service.trim()}. " +
                            "Les liens du service se règlent sur sa fiche, dans l'administration.",
                    )
                    OtButton("Terminé", onBack, Modifier.fillMaxWidth())
                }
                pending != null -> {
                    OtCard {
                        Text(
                            "Ajouter « ${pending.second} » au service « ${service.trim()} » ?",
                            style = MaterialTheme.typography.titleSmall,
                            color = Ot.colors.foreground,
                        )
                        Spacer(Modifier.height(6.dp))
                        Text(
                            "Vérifiez que ce nom est bien celui affiché sur l'écran que vous venez de scanner.",
                            style = MaterialTheme.typography.bodySmall,
                            color = Ot.colors.muted,
                        )
                    }
                    OtButton(
                        "Ajouter",
                        {
                            busy = true
                            error = null
                            scope.launch {
                                error = attempt { added = RelayHub.client(context).addServiceDevice(service.trim(), pending.first) }
                                busy = false
                            }
                        },
                        Modifier.fillMaxWidth().testTag("service-add"),
                        loading = busy,
                    )
                    OtButton("Annuler", { scanned = null }, Modifier.fillMaxWidth(), kind = ButtonKind.Secondary, enabled = !busy)
                }
                scanning -> {
                    Text(
                        "Visez le QR affiché par « ot pair join --qr --service » sur la machine du service.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = Ot.colors.muted,
                    )
                    QrScanner(
                        onResult = { text ->
                            when (val kind = classifyQr(text)) {
                                is QrKind.Sponsor -> {
                                    error = null
                                    scanning = false
                                    scanned = text to kind.name
                                }
                                is QrKind.Invalid -> error = kind.reason
                                else -> error = "Ce n'est pas le QR d'un appareil à ajouter."
                            }
                        },
                        modifier = Modifier.fillMaxWidth().aspectRatio(1f),
                    )
                    OtButton("Annuler", { scanning = false }, Modifier.fillMaxWidth(), kind = ButtonKind.Secondary)
                }
                else -> {
                    SectionLabel("Service")
                    OtTextField(service, { service = it.take(64) }, "Notifier", modifier = Modifier.testTag("service-name"))
                    if (existing.isNotEmpty()) {
                        Text("Services existants :", style = MaterialTheme.typography.bodySmall, color = Ot.colors.muted)
                        for (svc in existing) {
                            Text(
                                svc.name,
                                style = MaterialTheme.typography.bodyMedium,
                                color = Ot.colors.primaryText,
                                modifier = Modifier.clickable { service = svc.name }.padding(vertical = 4.dp),
                            )
                        }
                    }
                    Text(
                        "Sur la machine du service : ot pair join --qr --service --server <url> --name <appareil>",
                        style = MaterialTheme.typography.bodySmall.copy(fontFamily = Mono),
                        color = Ot.colors.muted,
                    )
                    OtButton(
                        "Scanner son QR",
                        { scanning = true },
                        Modifier.fillMaxWidth(),
                        enabled = service.isNotBlank(),
                    )
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}
