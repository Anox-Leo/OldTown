package dev.oldtown.android.ui

import dev.oldtown.android.ui.theme.OtTokens
import dev.oldtown.core.Member
import dev.oldtown.core.PersonDevice
import dev.oldtown.core.PersonItem
import dev.oldtown.core.Received
import java.text.Collator
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

enum class DeviceKind { PHONE, LAPTOP, DESKTOP, BROWSER, CLI, UNKNOWN, PERSON }

const val NO_CHANNEL = ""

const val USER_CHANNEL = "user:"

fun deviceKind(kind: String?): DeviceKind = when (kind) {
    "phone" -> DeviceKind.PHONE
    "laptop" -> DeviceKind.LAPTOP
    "desktop" -> DeviceKind.DESKTOP
    "browser" -> DeviceKind.BROWSER
    "cli" -> DeviceKind.CLI
    else -> DeviceKind.UNKNOWN
}

fun channelOf(item: Received, me: String?): String {
    val toUser = item.toUser
    if (toUser != null) return userChannel(toUser, item.toDevice)
    item.fromUser?.let { return userChannel(it) }
    val to = item.to
    if (item.outgoing) return to.singleOrNull() ?: NO_CHANNEL
    return if (to.size == 1 && to[0] == me) item.fromId else NO_CHANNEL
}

sealed interface ChannelTarget {
    data class Mine(val device: String) : ChannelTarget
    data class Person(val user: String, val device: String? = null) : ChannelTarget
}

fun parseChannel(channel: String): ChannelTarget {
    if (!channel.startsWith(USER_CHANNEL)) return ChannelTarget.Mine(channel)
    val rest = channel.removePrefix(USER_CHANNEL)
    val user = rest.substringBefore('/')
    val device = rest.substringAfter('/', "").ifEmpty { null }
    return ChannelTarget.Person(user, device)
}

fun userChannel(user: String, device: String? = null): String =
    if (device != null) "$USER_CHANNEL$user/$device" else "$USER_CHANNEL$user"

fun inChannel(item: Received, channel: String, me: String?): Boolean {
    val c = channelOf(item, me)
    return when (val t = parseChannel(channel)) {
        is ChannelTarget.Mine -> c == channel
        is ChannelTarget.Person -> if (t.device == null) {
            c == channel || c.startsWith("$channel/")
        } else {
            c == channel || (!item.outgoing && item.fromUser == t.user && item.fromId == t.device)
        }
    }
}

fun recipientsOf(channel: String): List<String> {
    require(channel != NO_CHANNEL) { "aucun destinataire : l'envoi à tous mes appareils n'existe plus" }
    return listOf(channel)
}

private val COLORS = OtTokens.avatarColors

fun avatarColor(id: String): Long {
    var h = 0x811c9dc5.toInt()
    for (c in id) {
        h = h xor c.code
        h *= 0x01000193
    }
    return COLORS[(h.toUInt() % COLORS.size.toUInt()).toInt()]
}

private val WORDS = Regex("""[\s\-_.]+""")

fun initials(name: String): String {
    val trimmed = name.trim()
    val words = trimmed.split(WORDS).filter { it.isNotEmpty() }
    val letters = if (words.size > 1) words[0].firstCodePoints(1) + words[1].firstCodePoints(1) else trimmed.firstCodePoints(2)
    return letters.uppercase(Locale.FRENCH)
}

private fun String.firstCodePoints(n: Int): String =
    substring(0, offsetByCodePoints(0, minOf(n, codePointCount(0, length))))

data class Conversation(
    val id: String,
    val name: String,
    val kind: DeviceKind,
    val color: Long,
    val last: Received?,
    val note: String? = null,
    val outdated: Boolean = false,
    val online: Boolean? = null,
)

private val collator: Collator = Collator.getInstance(Locale.FRENCH)
private val byName: Comparator<String> = Comparator { a, b -> collator.compare(a, b) }

private fun latest(items: Iterable<Received>): Received? = items.maxByOrNull { it.createdAtMs }

private fun lastByChannel(items: List<Received>, me: String?): Map<String, Received> {
    val last = HashMap<String, Received>()
    for (item in items) {
        val c = channelOf(item, me)
        val prev = last[c]
        if (prev == null || item.createdAtMs > prev.createdAtMs) last[c] = item
    }
    return last
}

fun conversations(
    members: List<Member>,
    items: List<Received>,
    me: String?,
    profiles: List<PersonDevice> = emptyList(),
): List<Conversation> {
    val profile = profiles.associateBy { it.deviceId }
    val last = lastByChannel(items, me)
    return members
        .filter { it.deviceId != me }
        .map { Conversation(it.deviceId, it.name, deviceKind(profile[it.deviceId]?.kind), avatarColor(it.deviceId), last[it.deviceId]) }
        .filter { it.kind != DeviceKind.CLI }
        .sortedWith(compareByDescending<Conversation> { it.last?.createdAtMs ?: 0L }.thenBy(byName) { it.name })
}

fun personConversations(person: PersonItem, items: List<Received>, me: String?): List<Conversation> {
    val channel = userChannel(person.userId)
    val mine = items.filter { inChannel(it, channel, me) }
    val head = Conversation(
        id = channel,
        name = person.name,
        kind = DeviceKind.PERSON,
        color = avatarColor(person.userId),
        last = latest(mine),
        note = if (person.isService) "service" else "ses appareils par défaut",
        online = person.devices.any { it.online },
    )
    if (person.isService) return listOf(head)
    val devices = person.devices
        .filter { it.reachable }
        .map { d ->
            val id = userChannel(person.userId, d.deviceId)
            val notes = listOfNotNull(
                "par défaut".takeIf { d.default },
                "pas à jour, ne recevra rien".takeIf { !d.upToDate },
            )
            Conversation(
                id = id,
                name = d.name,
                kind = deviceKind(d.kind),
                color = avatarColor(d.deviceId),
                last = latest(mine.filter { inChannel(it, id, me) }),
                note = notes.joinToString(" · ").ifEmpty { null },
                outdated = !d.upToDate,
                online = d.online,
            )
        }
        .sortedWith(compareBy<Conversation, String>(byName) { it.name })
    return listOf(head) + devices
}

fun mePerson(people: List<PersonItem>, myDevice: String?): PersonItem? =
    people.firstOrNull { p -> p.devices.any { it.deviceId == myDevice } }

private fun personOf(item: Received): String? = item.toUser ?: item.fromUser

fun railPeople(people: List<PersonItem>, items: List<Received>, meUser: String?): List<PersonItem> {
    val last = HashMap<String, Long>()
    for (i in items) {
        val p = personOf(i) ?: continue
        if (i.createdAtMs > (last[p] ?: 0L)) last[p] = i.createdAtMs
    }
    return people
        .filter { it.userId != meUser }
        .sortedWith(compareByDescending<PersonItem> { last[it.userId] ?: 0L }.thenBy(byName) { it.name })
}

fun unread(items: List<Received>, user: String, opened: Map<String, Long>): Int {
    val since = opened[user] ?: 0L
    return items.count { !it.outgoing && it.fromUser == user && it.createdAtMs > since }
}

data class ShareTarget(
    val channel: String,
    val name: String,
    val kind: DeviceKind,
    val color: Long,
)

fun shareTargets(
    members: List<Member>,
    items: List<Received>,
    me: String?,
    people: List<PersonItem> = emptyList(),
): List<ShareTarget> {
    val count = HashMap<String, Int>()
    val last = HashMap<String, Long>()
    for (item in items) {
        val c = item.toUser?.let { userChannel(it) } ?: item.fromUser?.let { userChannel(it) } ?: channelOf(item, me)
        count.merge(c, 1, Int::plus)
        if (item.createdAtMs > (last[c] ?: 0L)) last[c] = item.createdAtMs
    }
    val order = compareByDescending<ShareTarget> { count[it.channel] ?: 0 }
        .thenByDescending { last[it.channel] ?: 0L }
        .thenBy(byName) { it.name }
    val profiles = mePerson(people, me)?.devices.orEmpty().associateBy { it.deviceId }
    val devices = members
        .filter { it.deviceId != me }
        .map { ShareTarget(it.deviceId, it.name, deviceKind(profiles[it.deviceId]?.kind), avatarColor(it.deviceId)) }
        .filter { it.kind != DeviceKind.CLI }
        .sortedWith(order)
    val meUser = mePerson(people, me)?.userId
    val others = people
        .filter { it.userId != meUser && !it.isService }
        .map { ShareTarget(userChannel(it.userId), it.name, DeviceKind.PERSON, avatarColor(it.userId)) }
        .sortedWith(order)
    return devices + others
}

private val SPACES = Regex("""\s+""")

fun preview(item: Received): String {
    val body = item.body.replace(SPACES, " ").trim().take(200)
    return if (item.outgoing) "Vous : $body" else body
}

private val HOUR = DateTimeFormatter.ofPattern("HH:mm", Locale.FRENCH)
private val DAY = DateTimeFormatter.ofPattern("d MMM", Locale.FRENCH)

fun shortTime(ms: Long, now: Long = System.currentTimeMillis(), zone: ZoneId = ZoneId.systemDefault()): String {
    val at = Instant.ofEpochMilli(ms).atZone(zone)
    val sameDay = at.toLocalDate() == Instant.ofEpochMilli(now).atZone(zone).toLocalDate()
    return (if (sameDay) HOUR else DAY).format(at)
}
