package dev.oldtown.android

import android.content.Intent
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.share.ShareActivity
import dev.oldtown.core.forgetDevice
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Étape 3, T7 : la feuille de partage reçoit un `ACTION_SEND`. */
@RunWith(AndroidJUnit4::class)
class ShareActivityTest {
    @get:Rule val compose = createEmptyComposeRule()
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Before
    fun unpaired() {
        forgetDevice(RelayHub.store(context))
    }

    private fun share(type: String, text: String?) = Intent(context, ShareActivity::class.java)
        .setAction(Intent.ACTION_SEND)
        .setType(type)
        .apply { if (text != null) putExtra(Intent.EXTRA_TEXT, text) }

    @Test
    fun sharedLinkIsShownAndPairingIsRequired() {
        ActivityScenario.launch<ShareActivity>(share("text/plain", "https://example.org/article")).use {
            compose.waitUntil(TIMEOUT) { compose.onAllNodesWithTextSubstring("https://example.org/article").isNotEmpty() }
            compose.waitUntil(TIMEOUT) { compose.onAllNodesWithTextSubstring("pas encore appairé").isNotEmpty() }
        }
    }

    @Test
    fun imagesWaitForStage5() {
        ActivityScenario.launch<ShareActivity>(share("image/png", null)).use {
            // La première composition peut ne pas être faite : on attend le texte.
            compose.waitUntil(TIMEOUT) { compose.onAllNodesWithTextSubstring("Fichiers et images").isNotEmpty() }
        }
    }

    private companion object {
        const val TIMEOUT = 5_000L
    }

    private fun androidx.compose.ui.test.junit4.ComposeTestRule.onAllNodesWithTextSubstring(text: String) =
        onAllNodes(androidx.compose.ui.test.hasText(text, substring = true)).fetchSemanticsNodes()
}
