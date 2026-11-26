package dev.oldtown.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.oldtown.android.relay.HubState
import dev.oldtown.android.ui.components.Avatar
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.core.PersonItem

val RAIL_WIDTH = 72.dp

fun showRail(hub: HubState): Boolean {
    val me = hub.me ?: return false
    return me.isAdmin || hub.people.any { it.userId != me.userId }
}

@Composable
fun Rail(hub: HubState, selected: String?, onChoose: (String?) -> Unit, onInvite: () -> Unit) {
    val me = hub.me
    val others = railPeople(hub.people, hub.history, me?.userId)
    Column(
        Modifier
            .width(RAIL_WIDTH)
            .fillMaxHeight()
            .background(Ot.colors.groupedCard)
            .verticalScroll(rememberScrollState())
            .padding(vertical = 12.dp)
            .testTag("rail"),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        RailPerson(
            name = me?.name.orEmpty(),
            color = avatarColor(me?.userId.orEmpty()),
            selected = selected == null,
            unread = 0,
            online = false,
            description = "Mes appareils",
            onClick = { onChoose(null) },
        )
        HorizontalDivider(Modifier.width(28.dp), thickness = 2.dp, color = Ot.colors.border)
        for (p in others) {
            val n = unread(hub.history, p.userId, hub.opened)
            RailPerson(
                name = p.name,
                color = avatarColor(p.userId),
                selected = selected == p.userId,
                unread = n,
                online = p.devices.any { it.online },
                description = describe(p, n),
                onClick = { onChoose(p.userId) },
            )
        }
        if (hub.isAdmin) {
            Box(
                Modifier
                    .size(44.dp)
                    .clip(CircleShape)
                    .border(1.dp, Ot.colors.border, CircleShape)
                    .clickable(role = Role.Button, onClick = onInvite)
                    .testTag("rail-invite"),
                contentAlignment = Alignment.Center,
            ) {
                Icon(OtIcons.Plus, contentDescription = "Inviter une personne", tint = Ot.colors.muted, modifier = Modifier.size(20.dp))
            }
        }
    }
}

private fun describe(p: PersonItem, unread: Int): String {
    val kind = if (p.isService) " (service)" else ""
    val count = when (unread) {
        0 -> ""
        1 -> ", 1 non lu"
        else -> ", $unread non lus"
    }
    return p.name + kind + count
}

@Composable
private fun RailPerson(
    name: String,
    color: Long,
    selected: Boolean,
    unread: Int,
    online: Boolean,
    description: String,
    onClick: () -> Unit,
) {
    Box(
        Modifier
            .size(52.dp)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = description },
    ) {
        Box(
            Modifier
                .align(Alignment.Center)
                .size(48.dp)
                .clip(CircleShape)
                .border(2.dp, if (selected) Ot.colors.primary else Color.Transparent, CircleShape)
                .padding(2.dp),
            contentAlignment = Alignment.Center,
        ) {
            Avatar(DeviceKind.PERSON, color, size = 44.dp, online = online, initials = initials(name))
        }
        if (unread > 0) {
            Text(
                if (unread > 9) "9+" else unread.toString(),
                style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.SemiBold, fontSize = 11.sp),
                color = Ot.colors.screen,
                textAlign = TextAlign.Center,
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .widthIn(min = 18.dp)
                    .clip(CircleShape)
                    .background(Ot.colors.dangerForeground)
                    .padding(horizontal = 4.dp, vertical = 1.dp),
            )
        }
    }
}
