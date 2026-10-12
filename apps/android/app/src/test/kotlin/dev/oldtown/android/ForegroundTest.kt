package dev.oldtown.android

import dev.oldtown.android.notify.shouldNotify
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Étape 4, T8b : appli au premier plan → pas de notification. */
class ForegroundTest {
    @Test
    fun noNotificationWhileTheAppIsVisible() {
        assertFalse(shouldNotify(mainVisible = true))
    }

    @Test
    fun notificationOtherwise() {
        assertTrue(shouldNotify(mainVisible = false))
    }
}
