package dev.oldtown.android

import dev.oldtown.android.notify.oldestBeyond
import dev.oldtown.android.notify.shouldNotify
import org.junit.Assert.assertEquals
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
    fun noNotificationWhileAndroidSaysTheAppIsVisible() {
        // Le drapeau a manqué le passage au premier plan : Android le sait.
        assertFalse(shouldNotify(mainVisible = false, processForeground = true))
    }

    @Test
    fun notificationOtherwise() {
        assertTrue(shouldNotify(mainVisible = false))
    }

    @Test
    fun atMostThreeMessagesTheOldestLeaves() {
        val posted = listOf(10 to 300L, 11 to 100L, 12 to 400L, 13 to 200L)
        assertEquals(listOf(11), oldestBeyond(posted, 3))
        assertEquals(emptyList<Int>(), oldestBeyond(posted.take(3), 3))
        assertEquals(listOf(11, 13), oldestBeyond(posted, 2))
    }
}
