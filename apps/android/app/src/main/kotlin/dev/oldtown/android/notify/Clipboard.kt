package dev.oldtown.android.notify

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.PersistableBundle

object Clipboard {
    fun copy(context: Context, text: String, sensitive: Boolean) {
        val clip = ClipData.newPlainText("OldTown", text)
        if (sensitive) {
            clip.description.extras = PersistableBundle().apply {
                putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
            }
        }
        context.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
    }
}
