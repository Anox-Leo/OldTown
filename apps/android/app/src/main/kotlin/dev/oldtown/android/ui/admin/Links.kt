package dev.oldtown.android.ui.admin

import dev.oldtown.core.AdminUserItem

/** Lien entre deux personnes, tel que le relais le connaît (D41). */
enum class Link { BOTH, ONE_WAY, NONE }

/** Lien entre `a` et `b` : dans les deux sens, dans un seul, ou aucun (comme `linkBetween` de l'extension). */
fun linkBetween(users: List<AdminUserItem>, a: String, b: String): Link {
    val ab = users.firstOrNull { it.userId == a }?.links?.contains(b) ?: false
    val ba = users.firstOrNull { it.userId == b }?.links?.contains(a) ?: false
    return when {
        ab && ba -> Link.BOTH
        ab || ba -> Link.ONE_WAY
        else -> Link.NONE
    }
}

/** Personne encore là : invitée ou active, pas révoquée. */
fun AdminUserItem.present(): Boolean = state != "revoked"

/**
 * Liens montrés en interrupteurs sur la fiche d'une personne : les autres
 * personnes (sauf les services), dont le lien se pose dans les deux sens ;
 * pour un service, les personnes à qui il écrit (sens unique).
 */
fun linkCandidates(users: List<AdminUserItem>, user: String): List<AdminUserItem> =
    users.filter { it.userId != user && it.present() && it.role != "service" }

/** Services qui écrivent, ou pourraient écrire, à une personne. */
fun servicesFor(users: List<AdminUserItem>, user: String): List<AdminUserItem> =
    users.filter { it.userId != user && it.present() && it.role == "service" }
