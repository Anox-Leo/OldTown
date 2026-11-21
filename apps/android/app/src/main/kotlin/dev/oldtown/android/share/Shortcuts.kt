package dev.oldtown.android.share

import android.content.Context
import android.content.Intent
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import dev.oldtown.android.R
import dev.oldtown.android.ui.shareTargets
import dev.oldtown.core.Group
import dev.oldtown.core.PersonItem
import dev.oldtown.core.Received

/**
 * Direct Share : un raccourci par destinataire de la feuille de partage
 * (mes autres appareils, puis les autres personnes, note 0033 et étape 10),
 * mis à jour au démarrage du service et à chaque changement du groupe ou de
 * l'annuaire, dans l'ordre de [shareTargets]. Le menu Partager les propose
 * en tête.
 */
object Shortcuts {
    const val CATEGORY = "dev.oldtown.android.category.SHARE_TARGET"
    private const val PREFIX = "to:"

    fun update(context: Context, group: Group?, history: List<Received>, people: List<PersonItem>) {
        val members = group?.members.orEmpty()
        val targets = shareTargets(members, history, members.firstOrNull { it.isSelf }?.deviceId, people)
        val max = ShortcutManagerCompat.getMaxShortcutCountPerActivity(context).coerceAtLeast(1)
        val shortcuts = targets.take(max).mapIndexed { rank, t ->
            ShortcutInfoCompat.Builder(context, idOf(t.channel))
                .setShortLabel(t.name)
                .setLongLabel("Envoyer à ${t.name}")
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

    fun idOf(channel: String) = PREFIX + channel

    /** Canal visé par un raccourci Direct Share, `null` sinon. */
    fun targetOf(shortcutId: String?): String? = shortcutId?.removePrefix(PREFIX)?.takeIf { it != shortcutId }
}
