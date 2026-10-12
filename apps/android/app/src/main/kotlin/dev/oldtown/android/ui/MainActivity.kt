package dev.oldtown.android.ui

import android.Manifest
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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.oldtown.android.relay.Notice
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.components.Banner
import dev.oldtown.android.ui.components.BannerKind
import dev.oldtown.android.ui.theme.OldTownTheme
import dev.oldtown.android.ui.theme.Ot
import kotlinx.coroutines.delay

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent { OldTownTheme { App() } }
    }
}

/** Écrans au-dessus de l'accueil, dans l'ordre de la pile. */
sealed interface Screen {
    data object Group : Screen
    data object Settings : Screen
    data object Sponsor : Screen
    data object ShowCode : Screen
}

@Composable
private fun App() {
    val context = LocalContext.current
    val hub by RelayHub.state.collectAsStateWithLifecycle()
    val stack = remember { mutableStateListOf<Screen>() }
    var notice by remember { mutableStateOf<Notice?>(null) }

    LaunchedEffect(Unit) {
        RelayHub.reload(context)
        RelayHub.ensureStarted(context)
    }
    LaunchedEffect(Unit) { RelayHub.notices.collect { notice = it } }
    LaunchedEffect(notice) {
        if (notice != null) {
            delay(4_000)
            notice = null
        }
    }

    // Notifications : demandées une fois l'appareil appairé.
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
                "onboarding" -> OnboardingScreen()
                "home" -> HomeScreen(hub, open = { stack.add(it) })
                Screen.Group -> GroupScreen(hub, onBack = pop, open = { stack.add(it) })
                Screen.Settings -> SettingsScreen(hub, onBack = pop, onForgotten = { stack.clear() })
                Screen.Sponsor -> SponsorScreen(onBack = pop)
                Screen.ShowCode -> ShowCodeScreen(onBack = pop)
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
                }
            }
        }
    }
}
