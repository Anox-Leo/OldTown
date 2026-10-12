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
    fun filesWaitForStage5() {
        assertEquals(SharedContent.Unsupported, readShare(Intent.ACTION_SEND, "image/png", null))
        assertEquals(SharedContent.Unsupported, readShare(Intent.ACTION_SEND_MULTIPLE, "application/pdf", null))
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
