package dev.oldtown.android

import dev.oldtown.android.notify.MessageAction.COPY
import dev.oldtown.android.notify.MessageAction.OPEN
import dev.oldtown.android.notify.MessageAction.SHARE
import dev.oldtown.android.notify.actionsFor
import dev.oldtown.android.notify.isWebLink
import dev.oldtown.android.notify.preview
import dev.oldtown.core.ContentKind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Étape 3, T6 : choix des actions d'une notification selon le `kind`. */
class MessageActionsTest {
    @Test
    fun textGetsCopyAndShare() {
        assertEquals(listOf(COPY, SHARE), actionsFor(ContentKind.TEXT, "code 4821"))
        // Un texte qui ressemble à un lien reste un texte : pas d'« Ouvrir ».
        assertEquals(listOf(COPY, SHARE), actionsFor(ContentKind.TEXT, "https://example.org"))
    }

    @Test
    fun webLinkGetsOpen() {
        assertEquals(listOf(COPY, OPEN, SHARE), actionsFor(ContentKind.LINK, "https://example.org/a?b=c"))
        assertEquals(listOf(COPY, OPEN, SHARE), actionsFor(ContentKind.LINK, " HTTP://exemple.fr "))
    }

    @Test
    fun otherSchemesNeverOpen() {
        for (link in listOf("intent://x#Intent;end", "file:///sdcard/x", "javascript:alert(1)", "https://", "https://a b")) {
            assertEquals(link, listOf(COPY, SHARE), actionsFor(ContentKind.LINK, link))
        }
    }

    @Test
    fun webLinkDetection() {
        assertTrue(isWebLink("https://example.org"))
        assertTrue(isWebLink("http://192.168.1.2:8080/x"))
        assertFalse(isWebLink("ftp://example.org"))
        assertFalse(isWebLink("voir https://example.org"))
    }

    @Test
    fun previewIsOneShortLine() {
        assertEquals("a b c", preview("  a\n\n b\tc "))
        val long = "x".repeat(200)
        assertEquals(140, preview(long).length)
        assertTrue(preview(long).endsWith("…"))
    }
}
