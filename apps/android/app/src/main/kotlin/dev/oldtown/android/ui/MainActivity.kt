package dev.oldtown.android.ui

import android.Manifest
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.oldtown.android.notify.Foreground
import dev.oldtown.android.notify.Notifications
import dev.oldtown.android.relay.Notice
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.admin.AddServiceScreen
import dev.oldtown.android.ui.admin.AdminScreen
import dev.oldtown.android.ui.admin.InviteScreen
import dev.oldtown.android.ui.admin.PersonAdminScreen
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.theme.OldTownTheme
import dev.oldtown.android.ui.theme.Ot
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) openFrom(intent)
        setContent { OldTownTheme { App() } }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        openFrom(intent)
    }

    private fun openFrom(intent: Intent) {
        intent.getStringExtra(EXTRA_CHANNEL)?.let { openChannel.value = it }
        val data = intent.data
        if (intent.action == Intent.ACTION_VIEW && data?.scheme == "oldtown" && data.host == "invite") {
            openInvite.value = intent.dataString
        }
    }

    companion object {
        const val EXTRA_CHANNEL = "dev.oldtown.android.extra.CHANNEL"

        internal val openChannel = MutableStateFlow<String?>(null)

        internal val openInvite = MutableStateFlow<String?>(null)
    }

    override fun onStart() {
        super.onStart()
        Foreground.mainVisible = true
    }

    override fun onStop() {
        Foreground.mainVisible = false
        super.onStop()
    }
}

sealed interface Screen {
    data object Group : Screen
    data object Settings : Screen
    data object Sponsor : Screen
    data object ShowCode : Screen
    data class Conversation(val channel: String) : Screen
    data object MyDevices : Screen
    data object Admin : Screen
    data object Invite : Screen
    data class Person(val userId: String) : Screen
    data object AddService : Screen
}

@Composable
private fun App() {
    val context = LocalContext.current
    val hub by RelayHub.state.collectAsStateWithLifecycle()
    val stack = remember { mutableStateListOf<Screen>() }
    var notice by remember { mutableStateOf<Notice?>(null) }
    var person by rememberSaveable { mutableStateOf<String?>(null) }
    val invite by MainActivity.openInvite.collectAsStateWithLifecycle()

    LaunchedEffect(Unit) {
        RelayHub.reload(context)
        RelayHub.ensureStarted(context)
    }
    LaunchedEffect(Unit) { RelayHub.notices.collect { notice = it } }
    LaunchedEffect(Unit) {
        MainActivity.openChannel.collect { channel ->
            if (channel == null) return@collect
            stack.clear()
            person = (parseChannel(channel) as? ChannelTarget.Person)?.user
            stack.add(Screen.Conversation(channel))
            MainActivity.openChannel.value = null
        }
    }
    LaunchedEffect(notice) {
        if (notice != null) {
            delay(if (notice is Notice.PersonJoined) 12_000 else 4_000)
            notice = null
        }
    }
    LaunchedEffect(invite, hub.loaded, hub.device) {
        if (invite != null && hub.loaded && hub.device != null) {
            notice = Notice.Problem("Ce téléphone est déjà dans OldTown : l'invitation se scanne sur un nouvel appareil.")
            MainActivity.openInvite.value = null
        }
    }

    val notifLauncher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {}
    LaunchedEffect(hub.device != null) {
        if (hub.device != null && !SystemState.notificationsAllowed(context)) {
            notifLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    BackHandler(stack.isNotEmpty()) { stack.removeAt(stack.lastIndex) }
    val pop: () -> Unit = { if (stack.isNotEmpty()) stack.removeAt(stack.lastIndex) }

    Box(Modifier.fillMaxSize().background(Ot.colors.screen).safeDrawingPadding()) {
        val current: Any? = when {
            !hub.loaded -> null
            hub.device == null -> "onboarding"
            else -> stack.lastOrNull() ?: "home"
        }
        AnimatedContent(current, transitionSpec = { fadeIn() togetherWith fadeOut() }, label = "screen") { screen ->
            when (screen) {
                null -> Box(Modifier.fillMaxSize())
                "onboarding" -> OnboardingScreen(invite, onInviteRead = { MainActivity.openInvite.value = null })
                "home" -> HomeScreen(hub, person, onPerson = { person = it }, open = { stack.add(it) })
                Screen.Group -> GroupScreen(hub, onBack = pop, open = { stack.add(it) })
                Screen.Settings -> SettingsScreen(hub, onBack = pop, onForgotten = { stack.clear() }, open = { stack.add(it) })
                Screen.Sponsor -> SponsorScreen(onBack = pop)
                Screen.ShowCode -> ShowCodeScreen(onBack = pop)
                is Screen.Conversation -> ConversationScreen(hub, screen.channel, onBack = pop, onOpened = { channel ->
                    Notifications.clearRead(context, channel)
                    (parseChannel(channel) as? ChannelTarget.Person)?.let { RelayHub.markOpened(context, it.user) }
                })
                Screen.MyDevices -> MyDevicesScreen(hub, onBack = pop)
                Screen.Admin -> AdminScreen(hub, onBack = pop, open = { stack.add(it) })
                Screen.Invite -> InviteScreen(hub, onBack = pop)
                is Screen.Person -> PersonAdminScreen(hub, screen.userId, onBack = pop)
                Screen.AddService -> AddServiceScreen(hub, onBack = pop)
            }
        }
        notice?.let { n ->
            Box(
                Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(16.dp),
            ) {
                when (n) {
                    is Notice.PairingCompleted -> Banner("« ${n.name} » a rejoint le groupe.")
                    is Notice.JoinFailed -> Banner("Appairage : ${n.message}", BannerKind.Danger)
                    is Notice.Problem -> Banner(n.message, BannerKind.Warning)
                    is Notice.PersonJoined -> Banner(
                        "${n.name} a rejoint OldTown depuis « ${n.deviceName} ». Empreinte à comparer de vive voix : ${n.fingerprint}",
                    )
                    Notice.Joined -> Banner("Vous avez rejoint OldTown.")
                }
            }
        }
    }
}
