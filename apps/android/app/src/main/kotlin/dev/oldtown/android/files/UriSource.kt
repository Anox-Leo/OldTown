package dev.oldtown.android.files

import android.content.Context
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import dev.oldtown.core.FileSource
import dev.oldtown.core.StoreException
import java.io.Closeable
import java.io.FileInputStream
import java.io.IOException
import java.nio.ByteBuffer

/** Fichier partagé, décrit par son fournisseur. */
data class SharedFile(val uri: Uri, val name: String, val mime: String, val size: Long)

/** Nom, type et taille d'une URI `content://` partagée. */
fun describe(context: Context, uri: Uri): SharedFile {
    var name: String? = null
    var size: Long? = null
    context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)
        ?.use { c ->
            if (c.moveToFirst()) {
                c.getColumnIndex(OpenableColumns.DISPLAY_NAME).takeIf { it >= 0 && !c.isNull(it) }
                    ?.let { name = c.getString(it) }
                c.getColumnIndex(OpenableColumns.SIZE).takeIf { it >= 0 && !c.isNull(it) }
                    ?.let { size = c.getLong(it) }
            }
        }
    val mime = context.contentResolver.getType(uri) ?: "application/octet-stream"
    return SharedFile(uri, name ?: uri.lastPathSegment ?: "fichier", mime, size ?: -1)
}

/**
 * Lecture par morceaux d'une URI partagée, pour le cœur Rust (qui ne sait pas
 * ouvrir une URI `content://`). Lectures à un décalage : la reprise relit
 * seulement les morceaux manquants. Appelée hors du thread principal.
 */
class UriSource private constructor(
    private val pfd: ParcelFileDescriptor,
    private val length: Long,
) : FileSource, Closeable {
    private val channel = FileInputStream(pfd.fileDescriptor).channel

    override fun size(): ULong = length.toULong()

    override fun readAt(offset: ULong, len: UInt): ByteArray = synchronized(this) {
        try {
            val buf = ByteBuffer.allocate(len.toInt())
            var pos = offset.toLong()
            while (buf.hasRemaining()) {
                val n = channel.read(buf, pos)
                if (n < 0) break
                pos += n
            }
            buf.array().copyOf(buf.position())
        } catch (e: IOException) {
            throw StoreException.Failed("lecture du fichier : ${e.message}")
        }
    }

    override fun close() {
        channel.close()
        pfd.close()
    }

    companion object {
        /** Ouvre `uri` ; la taille vient du descripteur, sinon du fournisseur. */
        fun open(context: Context, file: SharedFile): UriSource {
            val pfd = context.contentResolver.openFileDescriptor(file.uri, "r")
                ?: throw IOException("fichier illisible")
            val size = pfd.statSize.takeIf { it >= 0 } ?: file.size.takeIf { it >= 0 }
            if (size == null) {
                pfd.close()
                throw IOException("taille du fichier inconnue")
            }
            return UriSource(pfd, size)
        }
    }
}
