package dev.oldtown.android.files

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import dev.oldtown.core.FileSink
import dev.oldtown.core.StoreException
import java.io.FileOutputStream
import java.io.IOException
import java.nio.ByteBuffer

/**
 * Fichier reçu, écrit dans `Download/OldTown` par MediaStore (aucune
 * permission de stockage). Il reste `IS_PENDING` (invisible des autres
 * applis) jusqu'à `finish`, appelé une fois le SHA-256 vérifié ; `abort`
 * l'efface.
 */
class DownloadSink private constructor(
    private val context: Context,
    private val uri: Uri,
) : FileSink {
    private val pfd = context.contentResolver.openFileDescriptor(uri, "rw")
        ?: throw IOException("fichier non créé")
    private val channel = FileOutputStream(pfd.fileDescriptor).channel

    override fun writeAt(offset: ULong, data: ByteArray) = synchronized(this) {
        try {
            val buf = ByteBuffer.wrap(data)
            var pos = offset.toLong()
            while (buf.hasRemaining()) pos += channel.write(buf, pos)
        } catch (e: IOException) {
            throw StoreException.Failed("écriture du fichier : ${e.message}")
        }
    }

    override fun finish(): String = synchronized(this) {
        try {
            channel.force(true)
            channel.close()
            pfd.close()
            val done = ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }
            context.contentResolver.update(uri, done, null, null)
            uri.toString()
        } catch (e: IOException) {
            throw StoreException.Failed("fichier non enregistré : ${e.message}")
        }
    }

    override fun abort() = synchronized(this) {
        runCatching { channel.close() }
        runCatching { pfd.close() }
        runCatching { context.contentResolver.delete(uri, null, null) }
        Unit
    }

    companion object {
        /** Crée l'entrée `Download/OldTown/<name>` (renommée par Android si le nom est pris). */
        fun create(context: Context, name: String, mime: String): DownloadSink {
            val values = ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, name)
                put(MediaStore.MediaColumns.MIME_TYPE, mime)
                put(MediaStore.MediaColumns.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/OldTown")
                put(MediaStore.MediaColumns.IS_PENDING, 1)
            }
            val uri = context.contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
                ?: throw IOException("Téléchargements inaccessible")
            return DownloadSink(context, uri)
        }
    }
}
