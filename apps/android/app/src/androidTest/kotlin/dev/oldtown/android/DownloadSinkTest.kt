package dev.oldtown.android

import android.provider.MediaStore
import androidx.core.net.toUri
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.oldtown.android.files.DownloadSink
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/** Étape 5, T5 : un fichier reçu s'écrit dans `Download/OldTown`, ou disparaît s'il est abandonné. */
@RunWith(AndroidJUnit4::class)
class DownloadSinkTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun finishedFileIsVisibleWithItsContent() {
        val sink = DownloadSink.create(context, "test-oldtown.bin", "application/octet-stream")
        sink.writeAt(3u, byteArrayOf(4, 5, 6))
        sink.writeAt(0u, byteArrayOf(1, 2, 3))
        val uri = sink.finish().toUri()
        try {
            val bytes = context.contentResolver.openInputStream(uri)!!.use { it.readBytes() }
            assertArrayEquals(byteArrayOf(1, 2, 3, 4, 5, 6), bytes)
            context.contentResolver.query(
                uri,
                arrayOf(MediaStore.MediaColumns.RELATIVE_PATH, MediaStore.MediaColumns.IS_PENDING),
                null,
                null,
                null,
            )!!.use { c ->
                c.moveToFirst()
                assertEquals("Download/OldTown/", c.getString(0))
                assertEquals(0, c.getInt(1))
            }
        } finally {
            context.contentResolver.delete(uri, null, null)
        }
    }

    @Test
    fun abortedFileIsRemoved() {
        val before = count()
        val sink = DownloadSink.create(context, "test-abandon.bin", "application/octet-stream")
        sink.writeAt(0u, byteArrayOf(1))
        sink.abort()
        assertEquals(before, count())
    }

    private fun count(): Int = context.contentResolver.query(
        MediaStore.Downloads.EXTERNAL_CONTENT_URI,
        arrayOf(MediaStore.MediaColumns._ID),
        "${MediaStore.MediaColumns.DISPLAY_NAME} LIKE 'test-abandon%'",
        null,
        null,
    )!!.use { it.count }
}
