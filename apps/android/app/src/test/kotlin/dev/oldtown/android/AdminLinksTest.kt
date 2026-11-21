package dev.oldtown.android

import dev.oldtown.android.ui.admin.Link
import dev.oldtown.android.ui.admin.linkBetween
import dev.oldtown.android.ui.admin.linkCandidates
import dev.oldtown.android.ui.admin.servicesFor
import dev.oldtown.core.AdminUserItem
import org.junit.Assert.assertEquals
import org.junit.Test

/** Mêmes cas que la grille du réseau de `people.spec.ts` (D41), plus le choix des interrupteurs. */
class AdminLinksTest {
    private fun user(id: String, links: List<String>, role: String = "member", state: String = "active") =
        AdminUserItem(userId = id, name = id, role = role, state = state, devices = emptyList(), links = links, muted = emptyList())

    @Test
    fun symmetricOrOneWayLinks() {
        val users = listOf(user("a", listOf("b")), user("b", listOf("a", "c")), user("c", emptyList()))
        assertEquals(Link.BOTH, linkBetween(users, "a", "b"))
        assertEquals(Link.BOTH, linkBetween(users, "b", "a"))
        assertEquals(Link.ONE_WAY, linkBetween(users, "b", "c"))
        assertEquals(Link.NONE, linkBetween(users, "a", "c"))
        assertEquals(Link.NONE, linkBetween(users, "a", "inconnu"))
    }

    @Test
    fun switchesLeaveOutRevokedPeopleAndServices() {
        val users = listOf(
            user("alice", emptyList(), role = "admin"),
            user("bob", emptyList()),
            user("carol", emptyList(), state = "revoked"),
            user("notifier", listOf("alice"), role = "service"),
            user("dan", emptyList(), state = "pending"),
        )
        assertEquals(listOf("alice", "dan"), linkCandidates(users, "bob").map { it.userId })
        assertEquals(listOf("notifier"), servicesFor(users, "bob").map { it.userId })
        assertEquals(listOf("alice", "bob", "dan"), linkCandidates(users, "notifier").map { it.userId })
    }
}
