package dev.oldtown.android.share

import android.content.Intent

/** Ce qu'une appli nous partage. */
sealed interface SharedContent {
    data class Text(val text: String) : SharedContent
    /** Fichiers, images… : pris en charge à l'étape 5. */
    data object Unsupported : SharedContent
    data object Empty : SharedContent
}

/**
 * Lit un `Intent` `ACTION_SEND`. Un texte partagé avec un sujet (titre d'une
 * page, par exemple) garde seulement le texte : le lien suffit.
 */
fun readShare(action: String?, type: String?, text: CharSequence?): SharedContent = when {
    action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE -> SharedContent.Empty
    type == null || !type.startsWith("text/") -> SharedContent.Unsupported
    text.isNullOrBlank() -> SharedContent.Empty
    else -> SharedContent.Text(text.toString())
}
