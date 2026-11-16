package dev.oldtown.android

import android.content.Intent
import dev.oldtown.android.share.SharedContent
import dev.oldtown.android.share.Shortcuts
import dev.oldtown.android.share.readShare
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Étape 3, T7 : lecture d'un `ACTION_SEND` et des raccourcis Direct Share. */
class SharedContentTest {
    @Test
    fun textIsAccepted() {
        assertEquals(
            SharedContent.Text("https://example.org"),
            readShare(Intent.ACTION_SEND, "text/plain", "https://example.org"),
        )
    }

    @Test
    fun filesAreAccepted() {
        val photo = "content://media/external/images/media/42"
        assertEquals(SharedContent.Files(listOf(photo)), readShare(Intent.ACTION_SEND, "image/png", null, listOf(photo)))
        val two = listOf(photo, "content://com.android.providers.downloads.documents/document/7")
        assertEquals(SharedContent.Files(two), readShare(Intent.ACTION_SEND_MULTIPLE, "*/*", null, two))
        // Un fichier texte partagé avec un texte : le fichier l'emporte.
        assertEquals(SharedContent.Files(listOf(photo)), readShare(Intent.ACTION_SEND, "text/plain", "x", listOf(photo)))
    }

    @Test
    fun onlyContentUrisAreSent() {
        // Une URI file:// pourrait désigner les clés de l'appli.
        val own = "file:///data/data/dev.oldtown.android/no_backup/device.key"
        assertEquals(SharedContent.Empty, readShare(Intent.ACTION_SEND, "application/octet-stream", null, listOf(own)))
        assertEquals(
            SharedContent.Files(listOf("content://a/1")),
            readShare(Intent.ACTION_SEND_MULTIPLE, "*/*", null, listOf(own, "content://a/1")),
        )
        // Une image sans flux : rien à envoyer.
        assertEquals(SharedContent.Empty, readShare(Intent.ACTION_SEND, "image/png", null))
    }

    @Test
    fun emptyOrForeignIntents() {
        assertEquals(SharedContent.Empty, readShare(Intent.ACTION_SEND, "text/plain", "  "))
        assertEquals(SharedContent.Empty, readShare(Intent.ACTION_VIEW, "text/plain", "x"))
    }

    @Test
    fun shortcutIds() {
        assertEquals("dev_01ABC", Shortcuts.memberOf("member:dev_01ABC"))
        assertNull(Shortcuts.memberOf("autre"))
        assertNull(Shortcuts.memberOf(null))
    }
}
