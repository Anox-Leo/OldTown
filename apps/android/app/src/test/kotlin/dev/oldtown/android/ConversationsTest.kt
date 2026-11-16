package dev.oldtown.android

import dev.oldtown.android.ui.ALL_NAME
import dev.oldtown.android.ui.DeviceKind
import dev.oldtown.android.ui.avatarColor
import dev.oldtown.android.ui.byUsage
import dev.oldtown.android.ui.channelOf
import dev.oldtown.android.ui.conversations
import dev.oldtown.android.ui.deviceKind
import dev.oldtown.android.ui.preview
import dev.oldtown.android.ui.recipientsOf
import dev.oldtown.android.ui.shortTime
import dev.oldtown.core.ContentKind
import dev.oldtown.core.Member
import dev.oldtown.core.Received
import java.time.ZoneId
import java.time.ZonedDateTime
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Mêmes cas que `devices.spec.ts` de l'extension (note 0031), plus le partage (note 0033). */
class ConversationsTest {
    private fun member(id: String, name: String) =
        Member(deviceId = id, name = name, signFingerprint = "", exchangeFingerprint = "", addedEpoch = 1u, isSelf = id == ME)

    private fun at(minute: Int) = ZonedDateTime.of(2026, 10, 15, 10, minute, 0, 0, ZoneId.of("UTC")).toInstant().toEpochMilli()

    private fun item(
        id: String,
        from: String,
        to: List<String>,
        minute: Int,
        outgoing: Boolean = false,
        body: String = id,
        fromName: String = from,
    ) = Received(
        msgId = id,
        fromId = from,
        fromName = fromName,
        createdAtMs = at(minute),
        kind = ContentKind.TEXT,
        body = body,
        sensitive = false,
        file = null,
        to = to,
        outgoing = outgoing,
        pending = false,
    )

    private fun sent(id: String, to: List<String>, minute: Int) = item(id, ME, to, minute, outgoing = true)

    @Test
    fun itemsGoToTheirChannel() {
        // Envoyé : son unique destinataire, sinon « Tous ».
        assertEquals("dev_a", channelOf(sent("1", listOf("dev_a"), 1), ME))
        assertEquals("", channelOf(sent("2", emptyList(), 1), ME))
        assertEquals("", channelOf(sent("3", listOf("dev_a", "dev_b"), 1), ME))
        // Reçu : l'expéditeur s'il m'était adressé à moi seul.
        assertEquals("dev_a", channelOf(item("4", "dev_a", listOf(ME), 1), ME))
        assertEquals("", channelOf(item("5", "dev_a", emptyList(), 1), ME))
        assertEquals("", channelOf(item("6", "dev_a", listOf(ME, "dev_b"), 1), ME))
        assertEquals(emptyList<String>(), recipientsOf(""))
        assertEquals(listOf("dev_a"), recipientsOf("dev_a"))
    }

    @Test
    fun guessesTheDeviceKindFromItsName() {
        assertEquals(DeviceKind.BROWSER, deviceKind("Chrome Mac"))
        assertEquals(DeviceKind.BROWSER, deviceKind("Brave Windows"))
        assertEquals(DeviceKind.PHONE, deviceKind("Téléphone"))
        assertEquals(DeviceKind.PHONE, deviceKind("Pixel 9"))
        assertEquals(DeviceKind.DESKTOP, deviceKind("PC Windows"))
        assertEquals(DeviceKind.DESKTOP, deviceKind("Mac mini"))
        assertEquals(DeviceKind.LAPTOP, deviceKind("MacBook Air"))
        assertEquals(DeviceKind.LAPTOP, deviceKind("relais"))
    }

    @Test
    fun sameColorAsTheExtension() {
        // Valeurs calculées par `avatarColor` de l'extension.
        assertEquals(0xFFE0803C, avatarColor(""))
        assertEquals(0xFFE05D5D, avatarColor("dev_a"))
        assertEquals(0xFFA35CC8, avatarColor("dev_b"))
        assertEquals(0xFFD6567F, avatarColor("dev_c"))
        assertEquals(0xFFC99A1E, avatarColor("dev_tel"))
        assertEquals(0xFF5F7D95, avatarColor("Téléphone"))
        assertEquals(0xFFE05D5D, avatarColor("01J9ZK3Q4W5E6R7T8Y9U0I1O2P"))
    }

    @Test
    fun allFirstThenMostRecentlyActive() {
        val list = conversations(MEMBERS, listOf(sent("1", listOf("dev_c"), 1), sent("2", listOf("dev_b"), 2), sent("3", emptyList(), 3)), ME)
        assertEquals(listOf(ALL_NAME, "Bravo", "Charlie", "Alpha"), list.map { it.name })
        assertEquals(listOf("3", "2", "1", null), list.map { it.last?.msgId })
        assertEquals(DeviceKind.GROUP, list[0].kind)
        assertEquals(avatarColor("dev_b"), list[1].color)
    }

    @Test
    fun previewOnOneLineWithItsAuthor() {
        val received = item("a", "dev_tel", listOf(ME), 1, body = "ligne 1\nligne 2", fromName = "Téléphone")
        assertEquals("ligne 1 ligne 2", preview(received, "dev_tel"))
        assertEquals("Téléphone : ligne 1 ligne 2", preview(received, ""))
        assertEquals("Vous : ligne 1 ligne 2", preview(received.copy(outgoing = true), ""))
    }

    @Test
    fun shareRecipientsByUsage() {
        val items = listOf(
            sent("1", listOf("dev_c"), 1),
            item("2", "dev_c", listOf(ME), 2),
            sent("3", listOf("dev_a"), 3),
            sent("4", listOf("dev_c"), 4),
            // À tous : ne compte pour aucun appareil.
            sent("5", emptyList(), 5),
            sent("6", emptyList(), 6),
            sent("7", listOf("dev_d"), 7),
        )
        val members = MEMBERS + member("dev_d", "Delta")
        // Charlie (3), puis Delta et Alpha (1 chacun, Delta plus récent), puis Bravo (0).
        assertEquals(listOf("Charlie", "Delta", "Alpha", "Bravo"), byUsage(members, items, ME).map { it.name })
        // Sans historique : par nom.
        assertEquals(listOf("Alpha", "Bravo", "Charlie"), byUsage(MEMBERS, emptyList(), ME).map { it.name })
    }

    @Test
    fun timeOfDayOrDate() {
        val utc = ZoneId.of("UTC")
        assertEquals("10:05", shortTime(at(5), now = at(30), zone = utc))
        val earlier = ZonedDateTime.of(2026, 10, 3, 9, 0, 0, 0, utc).toInstant().toEpochMilli()
        assertEquals("3 oct.", shortTime(earlier, now = at(30), zone = utc))
        assertNull(conversations(MEMBERS, emptyList(), ME)[1].last)
    }

    private companion object {
        const val ME = "dev_me"
        val MEMBERS = listOf("dev_me" to "Moi", "dev_a" to "Alpha", "dev_b" to "Bravo", "dev_c" to "Charlie")
            .map { (id, name) -> Member(id, name, "", "", 1u, id == ME) }
    }
}
