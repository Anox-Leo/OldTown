package dev.oldtown.android.ui

import dev.oldtown.core.Member
import dev.oldtown.core.Received
import java.text.Collator
import java.text.Normalizer
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

/**
 * Règles des conversations (plan `plan-android-conversations.md`, note
 * 0033), recopiées de l'extension (`devices.ts`, `channelOf` dans
 * `shared/api.ts`) pour que les deux se ressemblent : même rangement, même
 * type d'appareil, même couleur. Sans Android : testées sous JVM.
 */

/** Type d'appareil, pour l'avatar ; [GROUP] : la conversation « Tous ». */
enum class DeviceKind { PHONE, LAPTOP, DESKTOP, BROWSER, GROUP }

/** Nom de la conversation « Tous » (note 0031). */
const val ALL_NAME = "Tous mes appareils"

/** Canal de la conversation « Tous ». */
const val ALL_CHANNEL = ""

/**
 * Canal d'un élément : `""` pour « Tous », sinon l'appareil de la
 * conversation à deux. Envoyé : son unique destinataire. Reçu : son
 * expéditeur s'il m'était adressé à moi seul. Le reste va dans « Tous ».
 */
fun channelOf(item: Received, me: String?): String {
    val to = item.to
    if (item.outgoing) return to.singleOrNull() ?: ALL_CHANNEL
    return if (to.size == 1 && to[0] == me) item.fromId else ALL_CHANNEL
}

/** Destinataires d'un envoi dans un canal : vide pour « Tous ». */
fun recipientsOf(channel: String): List<String> = if (channel == ALL_CHANNEL) emptyList() else listOf(channel)

// Mots du nom, sans accents ni casse. Le roster ne dit pas le type de
// l'appareil (l'ajouter changerait le format) : il se devine au nom.
private val BROWSER =
    Regex("""\b(chrome|chromium|brave|firefox|edge|safari|opera|vivaldi|navigateur|browser|extension)\b""")
private val PHONE = Regex("""\b(telephone|phone|smartphone|mobile|android|pixel|iphone|galaxy|ipad|tablette)\b""")
private val DESKTOP = Regex("""\b(pc|desktop|fixe|tour|tower|windows|imac|mini|studio)\b""")
private val MARKS = Regex("""\p{M}+""")

/**
 * Type d'un appareil d'après son nom (note 0031) : navigateur d'abord
 * (« Chrome Mac » est l'extension), puis téléphone, ordinateur fixe ; un
 * portable sinon (Mac, Linux, ou un nom d'hôte).
 */
fun deviceKind(name: String): DeviceKind {
    val n = Normalizer.normalize(name, Normalizer.Form.NFD).replace(MARKS, "").lowercase(Locale.ROOT)
    return when {
        BROWSER.containsMatchIn(n) -> DeviceKind.BROWSER
        PHONE.containsMatchIn(n) -> DeviceKind.PHONE
        DESKTOP.containsMatchIn(n) -> DeviceKind.DESKTOP
        else -> DeviceKind.LAPTOP
    }
}

/** Couleurs d'avatar (ARGB), les mêmes que dans l'extension. */
private val COLORS = longArrayOf(
    0xFFE05D5D, 0xFFE0803C, 0xFFC99A1E, 0xFF3FA66B, 0xFF22988A,
    0xFF3B82C4, 0xFF6D6CF0, 0xFFA35CC8, 0xFFD6567F, 0xFF5F7D95,
)

/**
 * Couleur d'un appareil (ARGB) : tirée de son identifiant par FNV-1a 32 bits
 * sur ses caractères UTF-16, comme l'extension : la même des deux côtés.
 */
fun avatarColor(id: String): Long {
    var h = 0x811c9dc5.toInt()
    for (c in id) {
        h = h xor c.code
        h *= 0x01000193
    }
    return COLORS[(h.toUInt() % COLORS.size.toUInt()).toInt()]
}

data class Conversation(
    /** [ALL_CHANNEL] pour « Tous », sinon l'identifiant de l'appareil. */
    val id: String,
    val name: String,
    val kind: DeviceKind,
    val color: Long,
    /** Dernier élément reçu ou envoyé dans le canal. */
    val last: Received?,
)

private val byName: Comparator<String> = Collator.getInstance(Locale.FRENCH).let { c -> Comparator { a, b -> c.compare(a, b) } }

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
 * Liste de l'accueil (note 0031) : « Tous » en premier, puis un appareil par
 * ligne, du plus récemment actif au moins récent, puis par nom.
 */
fun conversations(members: List<Member>, items: List<Received>, me: String?): List<Conversation> {
    val last = lastByChannel(items, me)
    val devices = members
        .filter { it.deviceId != me }
        .map { Conversation(it.deviceId, it.name, deviceKind(it.name), avatarColor(it.deviceId), last[it.deviceId]) }
        .sortedWith(compareByDescending<Conversation> { it.last?.createdAtMs ?: 0L }.thenBy(byName) { it.name })
    val all = Conversation(ALL_CHANNEL, ALL_NAME, DeviceKind.GROUP, avatarColor(ALL_CHANNEL), last[ALL_CHANNEL])
    return listOf(all) + devices
}

/**
 * Destinataires de la feuille de partage (note 0033) : les autres appareils,
 * du plus utilisé au moins utilisé (éléments reçus et envoyés dans leur
 * conversation, dans l'historique), à égalité le plus récemment actif, puis
 * par nom.
 */
fun byUsage(members: List<Member>, items: List<Received>, me: String?): List<Member> {
    val count = HashMap<String, Int>()
    for (item in items) count.merge(channelOf(item, me), 1, Int::plus)
    val last = lastByChannel(items, me)
    return members
        .filter { it.deviceId != me }
        .sortedWith(
            compareByDescending<Member> { count[it.deviceId] ?: 0 }
                .thenByDescending { last[it.deviceId]?.createdAtMs ?: 0L }
                .thenBy(byName) { it.name },
        )
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
