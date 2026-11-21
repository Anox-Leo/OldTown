package dev.oldtown.android.share

import android.content.Intent
import dev.oldtown.android.files.sharedUris

sealed interface SharedContent {
    data class Text(val text: String) : SharedContent
    data class Files(val uris: List<String>) : SharedContent
    data object Empty : SharedContent
}

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
