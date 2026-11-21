package dev.oldtown.android.files

import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.Uri
import android.util.Log
import androidx.core.net.toUri
import dev.oldtown.android.notify.Notifications
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.relay.Transfer
import dev.oldtown.android.ui.ChannelTarget
import dev.oldtown.android.ui.parseChannel
import dev.oldtown.android.ui.recipientsOf
import dev.oldtown.core.OtClient
import dev.oldtown.core.OtException
import dev.oldtown.core.Received
import dev.oldtown.core.TransferListener
import java.io.IOException

object FileTransfers {
    private const val TAG = "OldTown"

    fun isUnmetered(context: Context): Boolean {
        val cm = context.getSystemService(ConnectivityManager::class.java)
        val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return false
        return caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
    }

    private class Progress(
        private val context: Context,
        private val transfer: Transfer,
    ) : TransferListener {
        override fun onProgress(done: ULong, total: ULong) {
            val t = transfer.copy(done = done.toLong(), total = total.toLong())
            RelayHub.setTransfer(t)
            Notifications.transferProgress(context, t)
        }
    }

    fun keepAccess(context: Context, uris: List<Uri>) {
        for (uri in uris) {
            runCatching {
                context.contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
        }
    }

    private fun release(context: Context, uri: Uri) {
        val held = context.contentResolver.persistedUriPermissions.any { it.uri == uri && it.isReadPermission }
        if (held) {
            runCatching {
                context.contentResolver.releasePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
        }
    }

    suspend fun send(context: Context, client: OtClient, uri: Uri, channel: String) {
        val id = uri.toString()
        var name = uri.lastPathSegment ?: "fichier"
        try {
            val file = describe(context, uri)
            name = file.name
            val start = Transfer(id, name, upload = true, done = 0, total = file.size.coerceAtLeast(0), channel = channel)
            RelayHub.setTransfer(start)
            UriSource.open(context, file).use { source ->
                val progress = Progress(context, start)
                when (val t = parseChannel(channel)) {
                    is ChannelTarget.Mine -> client.sendFile(source, id, file.name, file.mime, recipientsOf(t.device), progress)
                    is ChannelTarget.Person -> client.sendFileToUser(source, id, file.name, file.mime, t.user, t.device, progress)
                }
            }
            RelayHub.removeTransfer(id)
            Notifications.transferEnd(context, id)
            release(context, uri)
        } catch (e: OtException) {
            failed(context, Transfer(id, name, upload = true, 0, 0, channel = channel), e.message)
        } catch (e: IOException) {
            failed(context, Transfer(id, name, upload = true, 0, 0, channel = channel), e.message)
        } catch (e: SecurityException) {
            failed(context, Transfer(id, name, upload = true, 0, 0, channel = channel), "fichier plus accessible : le partager de nouveau")
        }
    }

    suspend fun download(context: Context, client: OtClient, item: Received): Received? {
        val file = item.file ?: return null
        if (file.savedUri != null) return item
        val start = Transfer(item.msgId, file.name, upload = false, done = 0, total = file.size.toLong())
        RelayHub.setTransfer(start)
        return try {
            val sink = DownloadSink.create(context, file.name, file.mime)
            val saved = client.download(item.msgId, sink, Progress(context, start))
            RelayHub.removeTransfer(item.msgId)
            Notifications.transferEnd(context, item.msgId)
            val updated = item.copy(file = saved)
            RelayHub.replace(updated)
            updated
        } catch (e: OtException) {
            failed(context, start, e.message)
            null
        } catch (e: IOException) {
            failed(context, start, e.message)
            null
        }
    }

    private fun failed(context: Context, transfer: Transfer, message: String?) {
        val t = transfer.copy(error = message ?: "transfert impossible")
        Log.w(TAG, "transfert en échec : ${t.error}")
        RelayHub.setTransfer(t)
        Notifications.transferFailed(context, t)
    }

    fun openIntent(savedUri: String, mime: String): Intent =
        Intent(Intent.ACTION_VIEW)
            .setDataAndType(savedUri.toUri(), mime)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)

    fun shareIntent(savedUri: String, mime: String): Intent {
        val uri = savedUri.toUri()
        val send = Intent(Intent.ACTION_SEND)
            .setType(mime)
            .putExtra(Intent.EXTRA_STREAM, uri)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        send.clipData = ClipData.newRawUri(null, uri)
        return Intent.createChooser(send, null)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
}
