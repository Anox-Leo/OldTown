package dev.oldtown.android.notify

import dev.oldtown.core.ContentKind

/** Action proposée sur un élément reçu. */
enum class MessageAction { COPY, OPEN, SHARE }

/**
 * Actions d'un élément reçu, dans l'ordre des boutons : Copier toujours,
 * Ouvrir pour un lien web seulement, Partager toujours. Fonction pure,
 * testée sans Android.
 */
fun actionsFor(kind: ContentKind, body: String): List<MessageAction> = buildList {
    add(MessageAction.COPY)
    if (kind == ContentKind.LINK && isWebLink(body)) add(MessageAction.OPEN)
    add(MessageAction.SHARE)
}

/**
 * Seuls les liens `http(s)://` s'ouvrent : un membre du groupe ne doit pas
 * pouvoir faire ouvrir `intent:`, `file:` ou une autre appli par un lien.
 */
fun isWebLink(text: String): Boolean {
    val t = text.trim()
    val lower = t.lowercase()
    val rest = when {
        lower.startsWith("https://") -> t.substring(8)
        lower.startsWith("http://") -> t.substring(7)
        else -> return false
    }
    return rest.isNotEmpty() && rest.none { it.isWhitespace() }
}

/** Aperçu d'un élément : une ligne, sans espaces superflus. */
fun preview(body: String, max: Int = 140): String {
    val oneLine = body.trim().replace(Regex("\\s+"), " ")
    return if (oneLine.length <= max) oneLine else oneLine.take(max - 1) + "…"
}
