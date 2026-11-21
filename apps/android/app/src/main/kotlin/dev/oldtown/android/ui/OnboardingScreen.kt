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
import androidx.compose.runtime.LaunchedEffect
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
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.components.ButtonKind
import dev.oldtown.android.ui.components.Logo
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
import dev.oldtown.core.joinInvite
import kotlinx.coroutines.launch

private enum class Step { Intro, Scan, Manual, Invite }

@Composable
fun OnboardingScreen(invite: String? = null, onInviteRead: () -> Unit = {}) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var step by rememberSaveable { mutableStateOf(Step.Intro) }
    var name by rememberSaveable { mutableStateOf(Build.MODEL.take(64)) }
    var server by rememberSaveable { mutableStateOf("wss://") }
    var code by remember { mutableStateOf("") }
    var link by rememberSaveable { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(invite) {
        if (invite != null) {
            link = invite
            step = Step.Invite
            onInviteRead()
        }
    }

    fun submitInvite(serverUrl: String, inviteCode: String) {
        busy = true
        error = null
        step = Step.Intro
        scope.launch {
            try {
                joinInvite(RelayHub.store(context), serverUrl, inviteCode, name)
                RelayHub.reload(context)
                RelayHub.ensureStarted(context)
            } catch (e: OtException) {
                error = e.message
            } finally {
                busy = false
            }
        }
    }

    fun submitLink() {
        when (val kind = classifyQr(link.trim())) {
            is QrKind.Invite -> submitInvite(kind.serverUrl, kind.code)
            is QrKind.Invalid -> error = kind.reason
            else -> error = "Ce n'est pas un lien d'invitation OldTown (oldtown://invite?…)."
        }
    }

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
        if (step != Step.Intro) {
            TopBar(if (step == Step.Invite) "J'ai reçu une invitation" else "Rejoindre un groupe", onBack = { step = Step.Intro })
        }
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            when (step) {
                Step.Intro -> {
                    Spacer(Modifier.height(48.dp))
                    Logo(36.dp)
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
                    OtButton(
                        "J'ai reçu une invitation",
                        { step = Step.Invite },
                        Modifier.fillMaxWidth().testTag("invite"),
                        kind = ButtonKind.Secondary,
                        icon = OtIcons.Link,
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
                        "Visez le QR affiché par « ot pair new --qr », ou le QR d'invitation.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = Ot.colors.muted,
                    )
                    QrScanner(
                        onResult = { text ->
                            when (val kind = classifyQr(text)) {
                                is QrKind.Join -> submit(kind.serverUrl, kind.code)
                                is QrKind.Invite -> submitInvite(kind.serverUrl, kind.code)
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
                Step.Invite -> {
                    Text(
                        "La personne qui vous invite vous a donné un QR ou un lien oldtown://invite. " +
                            "Il contient un secret : ne le partagez pas.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = Ot.colors.muted,
                    )
                    SectionLabel("Nom de ce téléphone")
                    OtTextField(name, { name = it.take(64) }, "Pixel")
                    SectionLabel("Lien d'invitation")
                    OtTextField(
                        link,
                        { link = it; error = null },
                        "oldtown://invite?…",
                        mono = true,
                        singleLine = false,
                        minLines = 2,
                        modifier = Modifier.testTag("invite-link"),
                    )
                    error?.let { Banner(it, BannerKind.Danger) }
                    OtButton(
                        "Rejoindre",
                        { submitLink() },
                        Modifier.fillMaxWidth(),
                        enabled = name.isNotBlank() && link.isNotBlank(),
                        loading = busy,
                    )
                    OtButton(
                        "Scanner le QR d'invitation",
                        { step = Step.Scan },
                        Modifier.fillMaxWidth(),
                        kind = ButtonKind.Secondary,
                        icon = OtIcons.Scan,
                        enabled = name.isNotBlank() && !busy,
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
