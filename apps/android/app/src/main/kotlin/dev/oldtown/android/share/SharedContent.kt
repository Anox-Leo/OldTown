package dev.oldtown.android.share

import android.content.Intent
import dev.oldtown.android.files.sharedUris

/** Ce qu'une appli nous partage. */
sealed interface SharedContent {
    data class Text(val text: String) : SharedContent
    /** Fichiers (étape 5) : URI `content://`, en texte. */
    data class Files(val uris: List<String>) : SharedContent
    data object Empty : SharedContent
}

/**
 * Lit un `Intent` `ACTION_SEND` ou `ACTION_SEND_MULTIPLE`. Des fichiers
 * (`EXTRA_STREAM`) l'emportent sur le texte qui les accompagne ; seules les
 * URI `content://` sont gardées. Un texte partagé avec un sujet (titre d'une
 * page, par exemple) garde seulement le texte : le lien suffit.
 */
fun readShare(
    action: String?,
    type: String?,
    text: CharSequence?,
    streams: List<String> = emptyList(),
): SharedContent {
    if (action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) return SharedContent.Empty
    val files = sharedUris(streams)
    return when {
        files.isNotEmpty() -> SharedContent.Files(files)
        type != null && type.startsWith("text/") && !text.isNullOrBlank() -> SharedContent.Text(text.toString())
        else -> SharedContent.Empty
    }
}
