package dev.oldtown.android.screens

import dev.oldtown.android.relay.Connection
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.ui.userChannel
import dev.oldtown.core.ContentKind
import dev.oldtown.core.DeviceState
import dev.oldtown.core.FileItem
import dev.oldtown.core.Group
import dev.oldtown.core.Member
import dev.oldtown.core.PersonDevice
import dev.oldtown.core.PersonItem
import dev.oldtown.core.Received

object Sample {
    const val ALICE = "u-alice"
    const val BOB = "u-bob"
    const val CAROLE = "u-carole"
    const val CODES = "u-codes"
    const val PHONE = "d-alice-phone"
    const val LAPTOP = "d-alice-laptop"
    const val DESKTOP = "d-alice-desktop"
    const val TERMINAL = "d-alice-cli"
    const val BOB_PHONE = "d-bob-phone"
    const val BOB_LAPTOP = "d-bob-laptop"

    private val now = System.currentTimeMillis()

    private fun ago(minutes: Long) = now - minutes * 60_000

    private fun device(id: String, name: String, kind: String?, default: Boolean = false, online: Boolean = false, reachable: Boolean = true) =
        PersonDevice(deviceId = id, name = name, kind = kind, reachable = reachable, default = default, upToDate = true, online = online)

    private fun member(id: String, name: String) =
        Member(deviceId = id, name = name, signFingerprint = "", exchangeFingerprint = "", addedEpoch = 1u, isSelf = id == PHONE)

    private fun text(
        id: String,
        minutesAgo: Long,
        body: String,
        from: String,
        fromName: String,
        outgoing: Boolean = false,
        to: List<String> = emptyList(),
        fromUser: String? = null,
        toUser: String? = null,
        kind: ContentKind = ContentKind.TEXT,
        file: FileItem? = null,
    ) = Received(
        msgId = id,
        fromId = from,
        fromName = fromName,
        createdAtMs = ago(minutesAgo),
        kind = kind,
        body = body,
        file = file,
        to = to,
        outgoing = outgoing,
        pending = false,
        fromUser = fromUser,
        toUser = toUser,
        toDevice = null,
    )

    val people = listOf(
        PersonItem(
            userId = ALICE,
            name = "Alice",
            isAdmin = true,
            isService = false,
            muted = false,
            devices = listOf(
                device(PHONE, "Téléphone", "phone", default = true, online = true),
                device(LAPTOP, "Portable", "laptop", online = true),
                device(DESKTOP, "Bureau", "desktop"),
                device(TERMINAL, "Terminal", "cli", reachable = false),
            ),
        ),
        PersonItem(
            userId = BOB,
            name = "Bob Martin",
            isAdmin = false,
            isService = false,
            muted = false,
            devices = listOf(
                device(BOB_PHONE, "Téléphone de Bob", "phone", default = true, online = true),
                device(BOB_LAPTOP, "Ordi de Bob", "laptop"),
            ),
        ),
        PersonItem(
            userId = CAROLE,
            name = "Carole",
            isAdmin = false,
            isService = false,
            muted = false,
            devices = listOf(device("d-carole-phone", "Téléphone", "phone", default = true)),
        ),
        PersonItem(
            userId = CODES,
            name = "Codes",
            isAdmin = false,
            isService = true,
            muted = false,
            devices = listOf(device("d-codes", "relais", null)),
        ),
    )

    val history = listOf(
        text("b5", 3, "Tu passes à quelle heure demain ?", BOB_PHONE, "Téléphone de Bob", fromUser = BOB, to = listOf(PHONE)),
        text("b4", 9, "Parfait, merci !", PHONE, "Téléphone", outgoing = true, toUser = BOB, to = listOf(BOB_PHONE)),
        text(
            "b3",
            12,
            "https://example.org/recette-tarte-aux-pommes",
            BOB_LAPTOP,
            "Ordi de Bob",
            fromUser = BOB,
            to = listOf(PHONE),
            kind = ContentKind.LINK,
        ),
        text(
            "b2",
            30,
            "",
            BOB_PHONE,
            "Téléphone de Bob",
            fromUser = BOB,
            to = listOf(PHONE),
            file = FileItem(blobId = "blob-1", name = "photos-vacances.zip", mime = "application/zip", size = 4_200_000u, savedUri = null),
        ),
        text("b1", 45, "Salut Alice, je t'envoie les photos du week-end.", BOB_PHONE, "Téléphone de Bob", fromUser = BOB, to = listOf(PHONE)),
        text("c1", 120, "Code de connexion : 482 913", "d-codes", "relais", fromUser = CODES, to = listOf(PHONE)),
        text("m2", 60, "Liste de courses : pain, œufs, café", LAPTOP, "Portable", to = listOf(PHONE)),
        text("m1", 200, "https://example.org/billets", PHONE, "Téléphone", outgoing = true, to = listOf(LAPTOP), kind = ContentKind.LINK),
        text("k1", 300, "Carole a rejoint OldTown", "d-carole-phone", "Téléphone", fromUser = CAROLE, to = listOf(PHONE)),
    )

    val hub = HubState(
        device = DeviceState(deviceId = PHONE, name = "Téléphone", serverUrl = "https://relais.example:8443"),
        connection = Connection.Connected(0u),
        group = Group(
            epoch = 3u,
            keyFingerprint = "",
            rosterVersion = 7u,
            members = listOf(
                member(PHONE, "Téléphone"),
                member(LAPTOP, "Portable"),
                member(DESKTOP, "Bureau"),
                member(TERMINAL, "Terminal"),
            ),
        ),
        history = history,
        loaded = true,
        people = people,
        opened = mapOf(BOB to ago(10), CAROLE to ago(400)),
    )

    val bobChannel = userChannel(BOB)
}
