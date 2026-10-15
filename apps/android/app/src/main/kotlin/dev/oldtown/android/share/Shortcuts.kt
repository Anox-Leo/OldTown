package dev.oldtown.android.share

import android.content.Context
import android.content.Intent
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import dev.oldtown.android.R
import dev.oldtown.android.ui.byUsage
import dev.oldtown.core.Group
import dev.oldtown.core.Received

/**
 * Direct Share : un raccourci par membre du groupe (sauf ce téléphone), mis
 * à jour au démarrage du service et à chaque changement du groupe, rangés
 * du plus utilisé au moins utilisé (note 0033). Le menu Partager les
 * propose en tête.
 */
object Shortcuts {
    const val CATEGORY = "dev.oldtown.android.category.SHARE_TARGET"
    private const val PREFIX = "member:"

    fun update(context: Context, group: Group?, history: List<Received>) {
        val all = group?.members.orEmpty()
        val members = byUsage(all, history, all.firstOrNull { it.isSelf }?.deviceId)
        val max = ShortcutManagerCompat.getMaxShortcutCountPerActivity(context).coerceAtLeast(1)
        val shortcuts = members.take(max).mapIndexed { rank, m ->
            ShortcutInfoCompat.Builder(context, idOf(m.deviceId))
                .setShortLabel(m.name)
                .setLongLabel("Envoyer à ${m.name}")
                .setIcon(IconCompat.createWithResource(context, R.drawable.ic_shortcut_device))
                .setCategories(setOf(CATEGORY))
                .setLongLived(true)
                .setRank(rank)
                .setIntent(
                    Intent(context, ShareActivity::class.java)
                        .setAction(Intent.ACTION_SEND)
                        .setType("text/plain"),
                )
                .build()
        }
        ShortcutManagerCompat.setDynamicShortcuts(context, shortcuts)
    }

    fun idOf(deviceId: String) = PREFIX + deviceId

    /** Membre visé par un raccourci Direct Share, `null` sinon. */
    fun memberOf(shortcutId: String?): String? = shortcutId?.removePrefix(PREFIX)?.takeIf { it != shortcutId }
}
