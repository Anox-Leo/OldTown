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

/**
 * Règles des conversations (plan `plan-android-conversations.md`, notes
 * 0033 et 0065), recopiées de l'extension (`devices.ts`, `channelOf` et
 * `inChannel` dans `shared/api.ts`) pour que les deux se ressemblent : même
 * rangement, même catégorie d'appareil, même couleur. Sans Android : testées
 * sous JVM, avec les cas de `people.spec.ts`.
 */

/**
 * Catégorie d'un appareil, pour l'avatar : celle qu'il déclare (note 0068),
 * [UNKNOWN] sans déclaration ; [GROUP] : « Tous » ; [PERSON] : une personne.
 */
enum class DeviceKind { PHONE, LAPTOP, DESKTOP, BROWSER, CLI, UNKNOWN, GROUP, PERSON }

/** Nom de la conversation « Tous » (note 0031). */
const val ALL_NAME = "Tous mes appareils"

/** Canal de la conversation « Tous ». */
const val ALL_CHANNEL = ""

/** Préfixe des canaux d'une autre personne : `user:<id>` ou `user:<id>/<appareil>`. */
const val USER_CHANNEL = "user:"

/** Catégorie déclarée (`phone`, `laptop`, `desktop`, `browser`, `cli`) ; inconnue ou absente : neutre. */
fun deviceKind(kind: String?): DeviceKind = when (kind) {
    "phone" -> DeviceKind.PHONE
    "laptop" -> DeviceKind.LAPTOP
    "desktop" -> DeviceKind.DESKTOP
    "browser" -> DeviceKind.BROWSER
    "cli" -> DeviceKind.CLI
    else -> DeviceKind.UNKNOWN
}

/**
 * Canal d'un élément. Vers une autre personne (envoi d'ici ou copie D43) :
 * `user:<id>`, ou `user:<id>/<appareil>` pour un appareil visé. Reçu d'une
 * autre personne : `user:<id>`. Entre mes appareils (note 0033) : `""` pour
 * « Tous », sinon l'appareil de la conversation à deux (envoyé : son unique
 * destinataire ; reçu : son expéditeur s'il m'était adressé à moi seul).
 */
fun channelOf(item: Received, me: String?): String {
    val toUser = item.toUser
    if (toUser != null) return userChannel(toUser, item.toDevice)
    item.fromUser?.let { return userChannel(it) }
    val to = item.to
    if (item.outgoing) return to.singleOrNull() ?: ALL_CHANNEL
    return if (to.size == 1 && to[0] == me) item.fromId else ALL_CHANNEL
}

/** Cible d'un canal : un de mes appareils (`""` : tous), ou une personne et peut-être un de ses appareils. */
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

/**
 * L'élément appartient-il au fil de `channel` ? Celui d'une personne montre
 * tout ce qui la concerne ; celui d'un de ses appareils, le même fil filtré :
 * envoyé à lui seul, ou reçu de lui.
 */
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

/** Destinataires d'un envoi dans un de mes canaux : vide pour « Tous ». */
fun recipientsOf(channel: String): List<String> = if (channel == ALL_CHANNEL) emptyList() else listOf(channel)

/** Couleurs d'avatar (ARGB), générées depuis `design/tokens.json` : les mêmes que dans l'extension. */
private val COLORS = OtTokens.avatarColors

/**
 * Couleur d'un appareil ou d'une personne (ARGB) : tirée de son identifiant
 * par FNV-1a 32 bits sur ses caractères UTF-16, comme l'extension : la même
 * des deux côtés.
 */
fun avatarColor(id: String): Long {
    var h = 0x811c9dc5.toInt()
    for (c in id) {
        h = h xor c.code
        h *= 0x01000193
    }
    return COLORS[(h.toUInt() % COLORS.size.toUInt()).toInt()]
}

private val WORDS = Regex("""[\s\-_.]+""")

/** Initiales d'une personne : la première lettre des deux premiers mots, sinon les deux premières du nom. */
fun initials(name: String): String {
    val trimmed = name.trim()
    val words = trimmed.split(WORDS).filter { it.isNotEmpty() }
    val letters = if (words.size > 1) words[0].firstCodePoints(1) + words[1].firstCodePoints(1) else trimmed.firstCodePoints(2)
    return letters.uppercase(Locale.FRENCH)
}

private fun String.firstCodePoints(n: Int): String =
    substring(0, offsetByCodePoints(0, minOf(n, codePointCount(0, length))))

data class Conversation(
    /** Canal : [ALL_CHANNEL], un de mes appareils, ou `user:…`. */
    val id: String,
    val name: String,
    val kind: DeviceKind,
    val color: Long,
    /** Dernier élément reçu ou envoyé dans le canal. */
    val last: Received?,
    /** Sous le nom : « par défaut », « pas à jour, ne recevra rien », « service »… */
    val note: String? = null,
    /** Appareil qui ne sait pas lire les messages entre personnes. */
    val outdated: Boolean = false,
    /** Présence lue dans l'annuaire (autres personnes) ; `null` : à demander au relais. */
    val online: Boolean? = null,
)

private val collator: Collator = Collator.getInstance(Locale.FRENCH)
private val byName: Comparator<String> = Comparator { a, b -> collator.compare(a, b) }

private fun latest(items: Iterable<Received>): Received? = items.maxByOrNull { it.createdAtMs }

/** Dernier élément de chaque canal. */
private fun lastByChannel(items: List<Received>, me: String?): Map<String, Received> {
    val last = HashMap<String, Received>()
    for (item in items) {
        val c = channelOf(item, me)
        val prev = last[c]
        if (prev == null || item.createdAtMs > prev.createdAtMs) last[c] = item
    }
    return last
}

/**
 * Accueil de ma personne (note 0031) : « Tous » en premier, puis un appareil
 * par ligne, du plus récemment actif au moins récent, puis par nom. Sa
 * catégorie vient de son profil dans l'annuaire (`profiles`) ; les appareils
 * en ligne de commande ne sont pas des conversations (A4).
 */
fun conversations(
    members: List<Member>,
    items: List<Received>,
    me: String?,
    profiles: List<PersonDevice> = emptyList(),
): List<Conversation> {
    val profile = profiles.associateBy { it.deviceId }
    val last = lastByChannel(items, me)
    val devices = members
        .filter { it.deviceId != me }
        .map { Conversation(it.deviceId, it.name, deviceKind(profile[it.deviceId]?.kind), avatarColor(it.deviceId), last[it.deviceId]) }
        .filter { it.kind != DeviceKind.CLI }
        .sortedWith(compareByDescending<Conversation> { it.last?.createdAtMs ?: 0L }.thenBy(byName) { it.name })
    val all = Conversation(ALL_CHANNEL, ALL_NAME, DeviceKind.GROUP, avatarColor(ALL_CHANNEL), last[ALL_CHANNEL])
    return listOf(all) + devices
}

/**
 * Accueil d'une autre personne : elle d'abord (ses appareils par défaut),
 * puis ses appareils joignables par nom. Un service n'a que sa
 * conversation : on ne lui répond pas (D41).
 */
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

/** Ma personne : celle dont un appareil est ce téléphone ; `null` sans annuaire. */
fun mePerson(people: List<PersonItem>, myDevice: String?): PersonItem? =
    people.firstOrNull { p -> p.devices.any { it.deviceId == myDevice } }

/** Personne d'un élément : destinataire d'un envoi (ou d'une copie), sinon expéditeur. */
private fun personOf(item: Received): String? = item.toUser ?: item.fromUser

/** Barre latérale : les autres personnes, de la plus récemment active à la moins, puis par nom. */
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

/** Non-lus d'une personne : reçus d'elle depuis la dernière ouverture de sa conversation. */
fun unread(items: List<Received>, user: String, opened: Map<String, Long>): Int {
    val since = opened[user] ?: 0L
    return items.count { !it.outgoing && it.fromUser == user && it.createdAtMs > since }
}

/** Destinataire de la feuille de partage : un de mes appareils, ou une personne. */
data class ShareTarget(
    /** Canal de l'envoi (comme [Conversation.id]). */
    val channel: String,
    val name: String,
    val kind: DeviceKind,
    val color: Long,
)

/**
 * Destinataires de la feuille de partage (note 0033) : mes autres appareils
 * (sans ceux en ligne de commande), puis les autres personnes (sans les
 * services, à qui on ne répond pas), chacun du plus utilisé au moins utilisé
 * (éléments de sa conversation dans l'historique), à égalité le plus
 * récemment actif, puis par nom.
 */
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

/**
 * Aperçu du dernier élément, sur une ligne : « Vous : » devant un envoi, le
 * nom de l'expéditeur devant un reçu de « Tous ».
 */
fun preview(item: Received, channel: String): String {
    val body = item.body.replace(SPACES, " ").trim().take(200)
    if (item.outgoing) return "Vous : $body"
    return if (channel == ALL_CHANNEL) "${item.fromName} : $body" else body
}

private val HOUR = DateTimeFormatter.ofPattern("HH:mm", Locale.FRENCH)
private val DAY = DateTimeFormatter.ofPattern("d MMM", Locale.FRENCH)

/** Heure d'un élément du jour (« 14:05 »), sa date sinon (« 3 oct. »), comme l'extension. */
fun shortTime(ms: Long, now: Long = System.currentTimeMillis(), zone: ZoneId = ZoneId.systemDefault()): String {
    val at = Instant.ofEpochMilli(ms).atZone(zone)
    val sameDay = at.toLocalDate() == Instant.ofEpochMilli(now).atZone(zone).toLocalDate()
    return (if (sameDay) HOUR else DAY).format(at)
}
