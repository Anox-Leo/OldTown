package dev.oldtown.android

import dev.oldtown.android.files.AUTO_DOWNLOAD_MAX
import dev.oldtown.android.files.FileAction.DOWNLOAD
import dev.oldtown.android.files.FileAction.OPEN
import dev.oldtown.android.files.FileAction.SHARE
import dev.oldtown.android.files.fileActions
import dev.oldtown.android.files.humanSize
import dev.oldtown.android.files.shouldAutoDownload
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Étape 5, T5 : téléchargement d'office et actions d'un fichier reçu. */
class FileRulesTest {
    @Test
    fun autoDownloadOnlySmallFilesOnUnmeteredNetworks() {
        assertTrue(shouldAutoDownload(1_000u, unmetered = true))
        assertTrue(shouldAutoDownload(AUTO_DOWNLOAD_MAX.toULong(), unmetered = true))
        assertFalse(shouldAutoDownload(AUTO_DOWNLOAD_MAX.toULong() + 1u, unmetered = true))
        // En 4G : jamais d'office, même petit.
        assertFalse(shouldAutoDownload(1_000u, unmetered = false))
    }

    @Test
    fun actionsDependOnTheFileBeingSaved() {
        assertEquals(listOf(DOWNLOAD), fileActions(saved = false))
        assertEquals(listOf(OPEN, SHARE), fileActions(saved = true))
    }

    @Test
    fun sizes() {
        assertEquals("512 o", humanSize(512))
        assertEquals("1,5 Mo", humanSize(1_572_864))
        assertEquals("20,0 Mo", humanSize(AUTO_DOWNLOAD_MAX))
    }
}
