package dev.oldtown.android

import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.ui.ALL_NAME
import dev.oldtown.android.ui.ConversationScreen
import dev.oldtown.android.ui.HomeScreen
import dev.oldtown.android.ui.theme.OldTownTheme
import dev.oldtown.core.ContentKind
import dev.oldtown.core.DeviceState
import dev.oldtown.core.Group
import dev.oldtown.core.Member
import dev.oldtown.core.Received
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Accueil et conversation (note 0033), sur un état fictif : sans relais ni
 * service, et sans toucher aux données de l'appli.
 */
@RunWith(AndroidJUnit4::class)
class ConversationsUiTest {
    @get:Rule val compose = createComposeRule()

    private fun item(id: String, from: String, to: List<String>, minutesAgo: Int, outgoing: Boolean) = Received(
        msgId = id,
        fromId = from,
        fromName = if (outgoing) "Moi" else from,
        createdAtMs = System.currentTimeMillis() - minutesAgo * 60_000L,
        kind = ContentKind.TEXT,
        body = "message $id",
        sensitive = false,
        file = null,
        to = to,
        outgoing = outgoing,
        pending = false,
    )

    private val hub = HubState(
        device = DeviceState(ME, "Moi", "ws://relais:7878"),
        group = Group(
            epoch = 1u,
            keyFingerprint = "",
            rosterVersion = 1u,
            members = listOf(ME to "Moi", "dev_a" to "Alpha", "dev_b" to "Bravo", "dev_c" to "Charlie")
                .map { (id, name) -> Member(id, name, "", "", 1u, id == ME) },
        ),
        history = listOf(
            item("3", ME, emptyList(), 1, outgoing = true),
            item("2", ME, listOf("dev_b"), 2, outgoing = true),
            item("1", "dev_c", listOf(ME), 3, outgoing = false),
        ),
        loaded = true,
    )

    private fun top(text: String) =
        compose.onNode(hasText(text) and hasTestTag("conversation-name"), useUnmergedTree = true)
            .fetchSemanticsNode().boundsInRoot.top

    @Test
    fun allFirstThenByRecency() {
        compose.setContent { OldTownTheme { HomeScreen(hub, open = {}) } }
        val tops = listOf(ALL_NAME, "Bravo", "Charlie", "Alpha").map(::top)
        assertEquals(tops.sorted(), tops)
        compose.onNodeWithText("Vous : message 3").assertExists()
    }

    @Test
    fun sentToADeviceIsOnTheRight() {
        compose.setContent { OldTownTheme { ConversationScreen(hub, channel = "dev_b", onBack = {}) } }
        val bubble = compose.onNodeWithText("message 2").fetchSemanticsNode().boundsInRoot
        val width = compose.onNode(hasTestTag("messages")).fetchSemanticsNode().boundsInRoot.width
        assertTrue("bulle à droite : $bubble sur $width", bubble.left > width / 2)
        // Les autres canaux n'y sont pas.
        compose.onNodeWithText("message 3").assertDoesNotExist()
        compose.onNodeWithText("message 1").assertDoesNotExist()
    }

    private companion object {
        const val ME = "dev_me"
    }
}
