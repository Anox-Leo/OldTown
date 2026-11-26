package dev.oldtown.android.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import com.github.takahirom.roborazzi.captureRoboImage
import com.github.takahirom.roborazzi.captureScreenRoboImage
import dev.oldtown.android.ui.ConversationScreen
import dev.oldtown.android.ui.HomeScreen
import dev.oldtown.android.ui.ItemSheet
import dev.oldtown.android.ui.LocalPresence
import dev.oldtown.android.ui.MyDevicesScreen
import dev.oldtown.android.ui.SettingsScreen
import dev.oldtown.android.ui.theme.OldTownTheme
import dev.oldtown.android.ui.theme.Ot
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [36], qualifiers = "fr-rFR-w411dp-h891dp-xxhdpi")
class ScreensCaptureTest(private val theme: String) {
    @get:Rule
    val compose = createComposeRule()

    private fun captureScreen(screen: String, content: @Composable () -> Unit) {
        compose.setContent {
            OldTownTheme(dark = theme == NIGHT) {
                CompositionLocalProvider(LocalPresence provides { null }) {
                    Box(Modifier.fillMaxSize().background(Ot.colors.screen)) { content() }
                }
            }
        }
        compose.waitForIdle()
        captureScreenRoboImage("build/outputs/roborazzi/$screen-$theme.png")
    }

    private fun capture(screen: String, content: @Composable () -> Unit) {
        compose.setContent {
            OldTownTheme(dark = theme == NIGHT) {
                CompositionLocalProvider(LocalPresence provides { null }) {
                    Box(Modifier.fillMaxSize().background(Ot.colors.screen)) { content() }
                }
            }
        }
        compose.onRoot().captureRoboImage("build/outputs/roborazzi/$screen-$theme.png")
    }

    @Test
    fun home() = capture("accueil") {
        HomeScreen(Sample.hub, null, onPerson = {}, open = {})
    }

    @Test
    fun homeOfAPerson() = capture("accueil-bob") {
        HomeScreen(Sample.hub, Sample.BOB, onPerson = {}, open = {})
    }

    @Test
    fun conversation() = capture("conversation") {
        ConversationScreen(Sample.hub, Sample.bobChannel, onBack = {}, onOpened = {})
    }

    @Test
    fun conversationBetweenMyDevices() = capture("conversation-portable") {
        ConversationScreen(Sample.hub, Sample.LAPTOP, onBack = {}, onOpened = {})
    }

    @Test
    fun linkSheet() = captureScreen("feuille-lien") {
        ItemSheet(Sample.history.first { it.msgId == "b3" }, onDismiss = {})
    }

    @Test
    fun myDevices() = capture("mes-appareils") {
        MyDevicesScreen(Sample.hub, onBack = {})
    }

    @Test
    fun settings() = capture("reglages") {
        SettingsScreen(Sample.hub, onBack = {}, onForgotten = {})
    }

    companion object {
        const val NIGHT = "nuit"
        const val PARCHMENT = "parchemin"

        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}")
        fun themes() = listOf(NIGHT, PARCHMENT)
    }
}
