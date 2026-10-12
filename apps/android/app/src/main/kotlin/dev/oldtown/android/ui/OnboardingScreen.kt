package dev.oldtown.android.ui

import android.os.Build
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
import androidx.compose.ui.unit.dp
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.components.ButtonKind
import dev.oldtown.android.ui.components.OtButton
import dev.oldtown.android.ui.components.OtCard
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.components.OtTextField
import dev.oldtown.android.ui.components.SectionLabel
import dev.oldtown.android.ui.components.TopBar
import dev.oldtown.android.ui.pairing.QrScanner
import dev.oldtown.android.ui.theme.Mono
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.core.OtException
import dev.oldtown.core.QrKind
import dev.oldtown.core.classifyQr
import dev.oldtown.core.join
import kotlinx.coroutines.launch

private enum class Step { Intro, Scan, Manual }

/**
 * Premier lancement : le téléphone rejoint le groupe en scannant le QR de
 * `ot pair new --qr` (ou en saisissant le code), puis attend le parrain.
 */
@Composable
fun OnboardingScreen() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var step by rememberSaveable { mutableStateOf(Step.Intro) }
    var name by rememberSaveable { mutableStateOf(Build.MODEL.take(64)) }
    var server by rememberSaveable { mutableStateOf("wss://") }
    var code by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }

    fun submit(serverUrl: String, pairingCode: String) {
        busy = true
        error = null
        step = Step.Intro
        scope.launch {
            try {
                join(RelayHub.store(context), serverUrl, pairingCode, name)
                RelayHub.reload(context)
                RelayHub.ensureStarted(context)
            } catch (e: OtException) {
                error = e.message
                step = Step.Intro
            } finally {
                busy = false
            }
        }
    }

    Column(Modifier.fillMaxSize()) {
        if (step != Step.Intro) TopBar("Rejoindre un groupe", onBack = { step = Step.Intro })
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            when (step) {
                Step.Intro -> {
                    Spacer(Modifier.height(48.dp))
                    Text("OldTown", style = MaterialTheme.typography.headlineSmall, color = Ot.colors.foreground)
                    Text(
                        "Textes et liens entre vos appareils, chiffrés de bout en bout. Le relais ne voit jamais le contenu.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = Ot.colors.muted,
                    )
                    Spacer(Modifier.height(12.dp))
                    SectionLabel("Nom de ce téléphone")
                    OtTextField(name, { name = it.take(64) }, "Pixel")
                    error?.let { Banner(it, BannerKind.Danger) }
                    Spacer(Modifier.height(4.dp))
                    OtButton(
                        "Scanner le QR",
                        { step = Step.Scan },
                        Modifier.fillMaxWidth(),
                        icon = OtIcons.Scan,
                        enabled = name.isNotBlank(),
                        loading = busy,
                    )
                    OtButton(
                        "Saisir le code",
                        { step = Step.Manual },
                        Modifier.fillMaxWidth(),
                        kind = ButtonKind.Secondary,
                        icon = OtIcons.Keyboard,
                        enabled = name.isNotBlank() && !busy,
                    )
                    Spacer(Modifier.height(8.dp))
                    OtCard {
                        Text("Sur un ordinateur du groupe", style = MaterialTheme.typography.labelMedium, color = Ot.colors.foreground)
                        Spacer(Modifier.height(6.dp))
                        Text(
                            "ot pair new --qr",
                            style = MaterialTheme.typography.bodyMedium.copy(fontFamily = Mono),
                            color = Ot.colors.foreground,
                        )
                        Spacer(Modifier.height(6.dp))
                        Text(
                            "puis scannez le QR affiché. Le code est valable 10 minutes et une seule fois.",
                            style = MaterialTheme.typography.bodySmall,
                            color = Ot.colors.muted,
                        )
                    }
                }
                Step.Scan -> {
                    Text(
                        "Visez le QR affiché par « ot pair new --qr ».",
                        style = MaterialTheme.typography.bodyMedium,
                        color = Ot.colors.muted,
                    )
                    QrScanner(
                        onResult = { text ->
                            when (val kind = classifyQr(text)) {
                                is QrKind.Join -> submit(kind.serverUrl, kind.code)
                                is QrKind.Sponsor -> {
                                    error = "Ce QR sert à ajouter un appareil au groupe : il se scanne depuis un téléphone déjà appairé."
                                    step = Step.Intro
                                }
                                is QrKind.Invalid -> {
                                    error = kind.reason
                                    step = Step.Intro
                                }
                                QrKind.Unknown -> {
                                    error = "Ce n'est pas un QR OldTown."
                                    step = Step.Intro
                                }
                            }
                        },
                        modifier = Modifier.fillMaxWidth().aspectRatio(1f),
                    )
                    OtButton(
                        "Saisir le code à la place",
                        { step = Step.Manual },
                        Modifier.fillMaxWidth(),
                        kind = ButtonKind.Secondary,
                    )
                }
                Step.Manual -> {
                    SectionLabel("URL du relais")
                    OtTextField(server, { server = it.trim() }, "wss://relais:7878", mono = true)
                    SectionLabel("Code d'appairage")
                    OtTextField(code, { code = it }, "ot1-…", mono = true, singleLine = false, minLines = 2)
                    Text(
                        "Affichés par « ot pair new » sur un ordinateur du groupe.",
                        style = MaterialTheme.typography.bodySmall,
                        color = Ot.colors.muted,
                    )
                    OtButton(
                        "Rejoindre",
                        { submit(server, code.trim()) },
                        Modifier.fillMaxWidth(),
                        enabled = server.length > 6 && code.isNotBlank(),
                        loading = busy,
                    )
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}
