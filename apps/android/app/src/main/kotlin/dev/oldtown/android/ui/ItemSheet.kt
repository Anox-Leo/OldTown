package dev.oldtown.android.ui

import android.content.Context
import android.content.Intent
import android.text.format.DateUtils
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import dev.oldtown.android.files.FileAction
import dev.oldtown.android.files.FileTransfers
import dev.oldtown.android.files.fileActions
import dev.oldtown.android.files.humanSize
import dev.oldtown.android.notify.Clipboard
import dev.oldtown.android.notify.MessageAction
import dev.oldtown.android.notify.Notifications
import dev.oldtown.android.notify.actionsFor
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.ui.components.ButtonKind
import dev.oldtown.android.ui.components.OtButton
import dev.oldtown.android.ui.components.OtIcons
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.android.ui.theme.OtShapes
import dev.oldtown.core.ContentKind
import dev.oldtown.core.Received

fun relativeTime(ms: Long): String {
    val now = System.currentTimeMillis()
    return if (now - ms < DateUtils.MINUTE_IN_MILLIS) {
        "à l'instant"
    } else {
        DateUtils.getRelativeTimeSpanString(ms, now, DateUtils.MINUTE_IN_MILLIS).toString()
    }
}

internal fun openFile(context: Context, uri: String, mime: String) {
    runCatching { context.startActivity(FileTransfers.openIntent(uri, mime)) }
}

/**
 * Feuille d'un élément : texte entier (sélectionnable) et ses actions, ou
 * fichier et ses actions. Un envoi d'ici est signé « Vous » ; un fichier
 * envoyé d'ici n'a pas d'action (ce téléphone n'en garde pas de copie).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ItemSheet(item: Received, onDismiss: () -> Unit) {
    val context = LocalContext.current
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        shape = OtShapes.sheet,
        containerColor = Ot.colors.card,
        scrimColor = Ot.colors.backdrop,
    ) {
        Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 28.dp)) {
            Text(
                "${if (item.outgoing) "Vous" else item.fromName}  ·  ${relativeTime(item.createdAtMs)}",
                style = MaterialTheme.typography.labelMedium,
                color = Ot.colors.muted,
            )
            Spacer(Modifier.height(10.dp))
            val file = item.file
            if (file != null) {
                FileSheetBody(item, file, onDismiss)
                return@Column
            }
            androidx.compose.foundation.text.selection.SelectionContainer {
                Text(
                    item.body,
                    style = MaterialTheme.typography.bodyLarge,
                    color = if (item.kind == ContentKind.LINK) Ot.colors.primaryText else Ot.colors.foreground,
                )
            }
            Spacer(Modifier.height(20.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (action in actionsFor(item.kind, item.body)) {
                    when (action) {
                        MessageAction.COPY -> OtButton("Copier", {
                            Clipboard.copy(context, item.body, item.sensitive)
                            onDismiss()
                        }, Modifier.weight(1f), icon = OtIcons.Copy)
                        MessageAction.OPEN -> OtButton("Ouvrir", {
                            openLink(context, item.body)
                            onDismiss()
                        }, Modifier.weight(1f), kind = ButtonKind.Secondary, icon = OtIcons.External)
                        MessageAction.SHARE -> OtButton("Partager", {
                            context.startActivity(Notifications.shareIntent(item.body))
                            onDismiss()
                        }, Modifier.weight(1f), kind = ButtonKind.Secondary, icon = OtIcons.Share)
                    }
                }
            }
        }
    }
}

@Composable
private fun FileSheetBody(item: Received, file: dev.oldtown.core.FileItem, onDismiss: () -> Unit) {
    val context = LocalContext.current
    Text(file.name, style = MaterialTheme.typography.bodyLarge, color = Ot.colors.foreground)
    Text(
        "${humanSize(file.size.toLong())} · ${file.mime}",
        style = MaterialTheme.typography.bodySmall,
        color = Ot.colors.muted,
    )
    if (item.outgoing) return
    Spacer(Modifier.height(20.dp))
    val saved = file.savedUri
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        for (action in fileActions(saved != null)) {
            when (action) {
                FileAction.DOWNLOAD -> OtButton("Télécharger", {
                    RelayHub.download(context, item.msgId)
                    onDismiss()
                }, Modifier.weight(1f), icon = OtIcons.Download)
                FileAction.OPEN -> OtButton("Ouvrir", {
                    if (saved != null) openFile(context, saved, file.mime)
                    onDismiss()
                }, Modifier.weight(1f), icon = OtIcons.External)
                FileAction.SHARE -> OtButton("Partager", {
                    if (saved != null) context.startActivity(FileTransfers.shareIntent(saved, file.mime))
                    onDismiss()
                }, Modifier.weight(1f), kind = ButtonKind.Secondary, icon = OtIcons.Share)
            }
        }
    }
}

private fun openLink(context: Context, link: String) {
    context.startActivity(
        Intent(Intent.ACTION_VIEW, link.trim().toUri()).addCategory(Intent.CATEGORY_BROWSABLE),
    )
}
