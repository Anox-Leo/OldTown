package dev.oldtown.android.notify

import dev.oldtown.core.ContentKind

enum class MessageAction { COPY, OPEN, SHARE }

fun actionsFor(kind: ContentKind, body: String): List<MessageAction> = buildList {
    add(MessageAction.COPY)
    if (kind == ContentKind.LINK && isWebLink(body)) add(MessageAction.OPEN)
    add(MessageAction.SHARE)
}

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

fun preview(body: String, max: Int = 140): String {
    val oneLine = body.trim().replace(Regex("\\s+"), " ")
    return if (oneLine.length <= max) oneLine else oneLine.take(max - 1) + "…"
}

fun senderTitle(person: String, device: String): String = if (device.isBlank() || device == person) person else "$person · $device"
