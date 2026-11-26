package dev.oldtown.android

import dev.oldtown.android.ui.ChannelTarget
import dev.oldtown.android.ui.DeviceKind
import dev.oldtown.android.ui.avatarColor
import dev.oldtown.android.ui.channelOf
import dev.oldtown.android.ui.conversations
import dev.oldtown.android.ui.deviceKind
import dev.oldtown.android.ui.inChannel
import dev.oldtown.android.ui.initials
import dev.oldtown.android.ui.parseChannel
import dev.oldtown.android.ui.personConversations
import dev.oldtown.android.ui.preview
import dev.oldtown.android.ui.railPeople
import dev.oldtown.android.ui.recipientsOf
import dev.oldtown.android.ui.shareTargets
import dev.oldtown.android.ui.shortTime
import dev.oldtown.android.ui.unread
import dev.oldtown.android.ui.userChannel
import dev.oldtown.core.ContentKind
import dev.oldtown.core.Member
import dev.oldtown.core.PersonDevice
import dev.oldtown.core.PersonItem
import dev.oldtown.core.Received
import java.time.ZoneId
import java.time.ZonedDateTime
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

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
        fromUser: String? = null,
        toUser: String? = null,
        toDevice: String? = null,
    ) = Received(
        msgId = id,
        fromId = from,
        fromName = fromName,
        createdAtMs = at(minute),
        kind = ContentKind.TEXT,
        body = body,
        file = null,
        to = to,
        outgoing = outgoing,
        pending = false,
        fromUser = fromUser,
        toUser = toUser,
        toDevice = toDevice,
    )

    private fun sent(id: String, to: List<String>, minute: Int) = item(id, ME, to, minute, outgoing = true)

    private fun device(
        id: String,
        name: String,
        kind: String? = null,
        reachable: Boolean = true,
        default: Boolean = false,
        upToDate: Boolean = true,
        online: Boolean = false,
    ) = PersonDevice(
        deviceId = id,
        name = name,
        kind = kind,
        reachable = reachable,
        default = default,
        upToDate = upToDate,
        online = online,
    )

    private fun person(user: String, name: String, devices: List<PersonDevice> = emptyList(), service: Boolean = false) =
        PersonItem(userId = user, name = name, isAdmin = false, isService = service, muted = false, devices = devices)

    @Test
    fun itemsGoToTheirChannel() {
        assertEquals("dev_a", channelOf(sent("1", listOf("dev_a"), 1), ME))
        assertEquals("", channelOf(sent("2", emptyList(), 1), ME))
        assertEquals("", channelOf(sent("3", listOf("dev_a", "dev_b"), 1), ME))
        assertEquals("dev_a", channelOf(item("4", "dev_a", listOf(ME), 1), ME))
        assertEquals("", channelOf(item("5", "dev_a", emptyList(), 1), ME))
        assertEquals("", channelOf(item("6", "dev_a", listOf(ME, "dev_b"), 1), ME))
        assertThrows(IllegalArgumentException::class.java) { recipientsOf("") }
        assertEquals(listOf("dev_a"), recipientsOf("dev_a"))
    }

    @Test
    fun receivedFromAnotherPersonGoesToHerConversation() {
        assertEquals("user:usr_bob", channelOf(item("1", "dev_bob", listOf(ME), 1, fromUser = BOB), ME))
    }

    @Test
    fun sentToAPersonOrToOneOfHerDevices() {
        val toBob = item("1", ME, listOf("dev_bob", "dev_tel"), 1, outgoing = true, toUser = BOB)
        assertEquals("user:usr_bob", channelOf(toBob, ME))
        assertEquals("user:usr_bob/dev_bob", channelOf(toBob.copy(toDevice = "dev_bob"), ME))
    }

    @Test
    fun aCopyFromOneOfMyDevicesGoesToThePerson() {
        val copy = item("1", "dev_tel", listOf("dev_bob", ME), 1, toUser = BOB)
        assertEquals("user:usr_bob", channelOf(copy, ME))
    }

    @Test
    fun betweenMyDevicesNothingChanges() {
        assertEquals("dev_tel", channelOf(item("1", "dev_tel", listOf(ME), 1), ME))
        assertEquals("", channelOf(item("2", "dev_tel", emptyList(), 1), ME))
        assertEquals("dev_tel", channelOf(item("3", ME, listOf("dev_tel"), 1, outgoing = true), ME))
    }

    @Test
    fun aChannelReadsBackAsATarget() {
        assertEquals(ChannelTarget.Mine(""), parseChannel(""))
        assertEquals(ChannelTarget.Mine("dev_tel"), parseChannel("dev_tel"))
        assertEquals(ChannelTarget.Person(BOB), parseChannel(userChannel(BOB)))
        assertEquals(ChannelTarget.Person(BOB, "dev_bob"), parseChannel(userChannel(BOB, "dev_bob")))
    }

    private val fromPhone = item("1", "dev_bob_tel", emptyList(), 1, fromUser = BOB)
    private val fromPc = item("2", "dev_bob_pc", emptyList(), 1, fromUser = BOB)
    private val toAll = item("3", ME, listOf("dev_bob_tel"), 1, outgoing = true, toUser = BOB)
    private val toPc = item("4", ME, emptyList(), 1, outgoing = true, toUser = BOB, toDevice = "dev_bob_pc")

    @Test
    fun thePersonConversationShowsEverything() {
        val c = userChannel(BOB)
        assertEquals(4, listOf(fromPhone, fromPc, toAll, toPc).count { inChannel(it, c, ME) })
    }

    @Test
    fun oneOfHerDevicesIsTheSameThreadFiltered() {
        val c = userChannel(BOB, "dev_bob_pc")
        assertEquals(listOf("2", "4"), listOf(fromPhone, fromPc, toAll, toPc).filter { inChannel(it, c, ME) }.map { it.msgId })
    }

    @Test
    fun anotherPersonStaysOut() {
        val carol = item("5", "dev_carol", emptyList(), 1, fromUser = CAROL)
        assertFalse(inChannel(carol, userChannel(BOB), ME))
        assertFalse(inChannel(carol, "", ME))
    }

    @Test
    fun aDeclaredKindIsKeptOtherwiseNeutral() {
        assertEquals(DeviceKind.PHONE, deviceKind("phone"))
        assertEquals(DeviceKind.CLI, deviceKind("cli"))
        assertEquals(DeviceKind.UNKNOWN, deviceKind("unknown"))
        assertEquals(DeviceKind.UNKNOWN, deviceKind(null))
    }

    @Test
    fun commandLineDevicesAreNotConversations() {
        val list = conversations(
            listOf(member(ME, "moi"), member("dev_tel", "tel"), member("dev_cli", "serveur")),
            emptyList(),
            ME,
            listOf(device("dev_cli", "serveur", kind = "cli")),
        )
        assertEquals(listOf("dev_tel"), list.map { it.id })
        assertEquals(DeviceKind.UNKNOWN, list[0].kind)
    }

    @Test
    fun anOldItemSentToAllEntersNoConversation() {
        val toAll = item("tous", "dev_tel", emptyList(), 9)
        val list = conversations(listOf(member(ME, "moi"), member("dev_tel", "tel")), listOf(toAll), ME)
        assertEquals(listOf("dev_tel"), list.map { it.id })
        assertNull(list[0].last)
    }

    @Test
    fun aPersonFirstThenHerReachableDevicesByName() {
        val bob = person(
            BOB,
            "Bob",
            listOf(
                device("dev_b2", "bob-pc"),
                device("dev_b1", "bob-phone", default = true, online = true),
                device("dev_b3", "bob-vieux", upToDate = false),
                device("dev_b4", "bob-cache", reachable = false),
            ),
        )
        val last = item("x", "dev_b2", emptyList(), 5, fromUser = BOB)
        val list = personConversations(bob, listOf(last), ME)
        assertEquals(
            listOf("user:usr_bob", "user:usr_bob/dev_b2", "user:usr_bob/dev_b1", "user:usr_bob/dev_b3"),
            list.map { it.id },
        )
        assertEquals("Bob", list[0].name)
        assertEquals(DeviceKind.PERSON, list[0].kind)
        assertEquals(true, list[0].online)
        assertEquals("x", list[0].last?.msgId)
        assertEquals("x", list[1].last?.msgId)
        assertNull(list[2].last)
        assertEquals("par défaut", list[2].note)
        assertEquals(true, list[3].outdated)
        assertEquals("pas à jour, ne recevra rien", list[3].note)
    }

    @Test
    fun aServiceHasOnlyItsConversation() {
        val svc = person("usr_svc", "Notifier", listOf(device("dev_n", "notifier-1")), service = true)
        assertEquals(listOf("user:usr_svc"), personConversations(svc, emptyList(), ME).map { it.id })
    }

    private val people = listOf(person("usr_me", "Alice"), person(BOB, "Bob"), person(CAROL, "Carol"), person("usr_dan", "Dan"))

    @Test
    fun otherPeopleMostRecentlyActiveFirstThenByName() {
        val items = listOf(
            item("1", "dev_bob", emptyList(), 1, fromUser = BOB),
            item("2", ME, emptyList(), 2, outgoing = true, toUser = "usr_dan"),
        )
        assertEquals(listOf("Dan", "Bob", "Carol"), railPeople(people, items, "usr_me").map { it.name })
    }

    @Test
    fun unreadSinceTheLastOpening() {
        val items = listOf(
            item("1", "dev_bob", emptyList(), 1, fromUser = BOB),
            item("2", "dev_bob", emptyList(), 3, fromUser = BOB),
            item("3", ME, emptyList(), 4, outgoing = true, toUser = BOB),
            item("4", "dev_carol", emptyList(), 4, fromUser = CAROL),
        )
        assertEquals(2, unread(items, BOB, emptyMap()))
        assertEquals(1, unread(items, BOB, mapOf(BOB to at(2))))
        assertEquals(0, unread(items, BOB, mapOf(BOB to at(9))))
    }

    @Test
    fun initialsOfAName() {
        assertEquals("AL", initials("Alice"))
        assertEquals("JP", initials("Jean-Paul"))
        assertEquals("MC", initials("Marie Claire Dupont"))
        assertEquals("ÉL", initials("élise"))
    }

    @Test
    fun sameColorAsTheExtension() {
        assertEquals(0xFFC77A1C, avatarColor(""))
        assertEquals(0xFFC4534F, avatarColor("dev_a"))
        assertEquals(0xFF9058B0, avatarColor("dev_b"))
        assertEquals(0xFFBE4F75, avatarColor("dev_c"))
        assertEquals(0xFFA68A22, avatarColor("dev_tel"))
        assertEquals(0xFF5A7488, avatarColor("Téléphone"))
        assertEquals(0xFFC4534F, avatarColor("01J9ZK3Q4W5E6R7T8Y9U0I1O2P"))
    }

    @Test
    fun mostRecentlyActiveFirstWithoutAllMyDevices() {
        val list = conversations(MEMBERS, listOf(sent("1", listOf("dev_c"), 1), sent("2", listOf("dev_b"), 2), sent("3", emptyList(), 3)), ME)
        assertEquals(listOf("Bravo", "Charlie", "Alpha"), list.map { it.name })
        assertEquals(listOf("2", "1", null), list.map { it.last?.msgId })
        assertEquals(avatarColor("dev_b"), list[0].color)
    }

    @Test
    fun previewOnOneLine() {
        val received = item("a", "dev_tel", listOf(ME), 1, body = "ligne 1\nligne 2", fromName = "Téléphone")
        assertEquals("ligne 1 ligne 2", preview(received))
        assertEquals("Vous : ligne 1 ligne 2", preview(received.copy(outgoing = true)))
    }

    @Test
    fun shareRecipientsByUsage() {
        val items = listOf(
            sent("1", listOf("dev_c"), 1),
            item("2", "dev_c", listOf(ME), 2),
            sent("3", listOf("dev_a"), 3),
            sent("4", listOf("dev_c"), 4),
            sent("5", emptyList(), 5),
            sent("6", emptyList(), 6),
            sent("7", listOf("dev_d"), 7),
        )
        val members = MEMBERS + member("dev_d", "Delta")
        assertEquals(listOf("Charlie", "Delta", "Alpha", "Bravo"), shareTargets(members, items, ME).map { it.name })
        assertEquals(listOf("Alpha", "Bravo", "Charlie"), shareTargets(MEMBERS, emptyList(), ME).map { it.name })
    }

    @Test
    fun shareMyDevicesThenPeopleWithoutCommandLineNorServices() {
        val mine = person("usr_me", "Alice", listOf(device(ME, "moi"), device("dev_c", "serveur", kind = "cli")))
        val all = listOf(
            mine,
            person(BOB, "Bob"),
            person(CAROL, "Carol"),
            person("usr_svc", "Notifier", service = true),
        )
        val items = listOf(
            item("1", "dev_carol", listOf(ME), 1, fromUser = CAROL),
            item("2", ME, listOf("dev_bob"), 2, outgoing = true, toUser = BOB, toDevice = "dev_bob"),
            item("3", ME, listOf("dev_carol"), 3, outgoing = true, toUser = CAROL),
        )
        val targets = shareTargets(MEMBERS, items, ME, all)
        assertEquals(listOf("Alpha", "Bravo", "Carol", "Bob"), targets.map { it.name })
        assertEquals(listOf("dev_a", "dev_b", "user:usr_carol", "user:usr_bob"), targets.map { it.channel })
        assertEquals(DeviceKind.PERSON, targets[2].kind)
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
        const val BOB = "usr_bob"
        const val CAROL = "usr_carol"
        val MEMBERS = listOf("dev_me" to "Moi", "dev_a" to "Alpha", "dev_b" to "Bravo", "dev_c" to "Charlie")
            .map { (id, name) -> Member(id, name, "", "", 1u, id == ME) }
    }
}
