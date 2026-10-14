package dev.oldtown.android

import android.content.Intent
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.share.ShareActivity
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Étape 3, T7 et étape 5, T5 : la feuille de partage reçoit un `ACTION_SEND`. */
@RunWith(AndroidJUnit4::class)
class ShareActivityTest {
    @get:Rule val compose = createEmptyComposeRule()
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private fun share(type: String, text: String?) = Intent(context, ShareActivity::class.java)
        .setAction(Intent.ACTION_SEND)
        .setType(type)
        .apply { if (text != null) putExtra(Intent.EXTRA_TEXT, text) }

    @Test
    fun sharedLinkIsShownAndPairingIsRequired() {
        assumeNoRealDevice(context)
        ActivityScenario.launch<ShareActivity>(share("text/plain", "https://example.org/article")).use {
            compose.waitUntil(TIMEOUT) { compose.onAllNodesWithTextSubstring("https://example.org/article").isNotEmpty() }
            compose.waitUntil(TIMEOUT) { compose.onAllNodesWithTextSubstring("pas encore appairé").isNotEmpty() }
        }
    }

    @Test
    fun imageWithoutStreamIsNothingToSend() {
        ActivityScenario.launch<ShareActivity>(share("image/png", null)).use {
            // La première composition peut ne pas être faite : on attend le texte.
            compose.waitUntil(TIMEOUT) { compose.onAllNodesWithTextSubstring("Rien à envoyer").isNotEmpty() }
        }
    }

    @Test
    fun sharedImageIsListedWithItsSize() {
        // Une image rangée par l'appli elle-même dans Téléchargements, partagée par son URI.
        val values = android.content.ContentValues().apply {
            put(android.provider.MediaStore.MediaColumns.DISPLAY_NAME, "partage-test.png")
            put(android.provider.MediaStore.MediaColumns.MIME_TYPE, "image/png")
        }
        val resolver = context.contentResolver
        val uri = resolver.insert(android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)!!
        try {
            resolver.openOutputStream(uri)!!.use { it.write(ByteArray(2048)) }
            val intent = share("image/png", null).putExtra(Intent.EXTRA_STREAM, uri)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            ActivityScenario.launch<ShareActivity>(intent).use {
                compose.waitUntil(TIMEOUT) { compose.onAllNodesWithTextSubstring("partage-test").isNotEmpty() }
                compose.waitUntil(TIMEOUT) { compose.onAllNodesWithTextSubstring("2,0 Ko").isNotEmpty() }
            }
        } finally {
            resolver.delete(uri, null, null)
        }
    }

    private companion object {
        const val TIMEOUT = 5_000L
    }

    private fun androidx.compose.ui.test.junit4.ComposeTestRule.onAllNodesWithTextSubstring(text: String) =
        onAllNodes(androidx.compose.ui.test.hasText(text, substring = true)).fetchSemanticsNodes()
}
