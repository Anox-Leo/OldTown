package dev.oldtown.android.ui.admin

import dev.oldtown.core.AdminUserItem

enum class Link { BOTH, ONE_WAY, NONE }

fun linkBetween(users: List<AdminUserItem>, a: String, b: String): Link {
    val ab = users.firstOrNull { it.userId == a }?.links?.contains(b) ?: false
    val ba = users.firstOrNull { it.userId == b }?.links?.contains(a) ?: false
    return when {
        ab && ba -> Link.BOTH
        ab || ba -> Link.ONE_WAY
        else -> Link.NONE
    }
}

fun AdminUserItem.present(): Boolean = state != "revoked"

fun linkCandidates(users: List<AdminUserItem>, user: String): List<AdminUserItem> =
    users.filter { it.userId != user && it.present() && it.role != "service" }

fun servicesFor(users: List<AdminUserItem>, user: String): List<AdminUserItem> =
    users.filter { it.userId != user && it.present() && it.role == "service" }
