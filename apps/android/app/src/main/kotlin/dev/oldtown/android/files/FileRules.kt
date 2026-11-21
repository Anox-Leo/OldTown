package dev.oldtown.android.files

const val AUTO_DOWNLOAD_MAX: Long = 20L * 1024 * 1024

fun shouldAutoDownload(size: ULong, unmetered: Boolean): Boolean =
    unmetered && size <= AUTO_DOWNLOAD_MAX.toULong()

enum class FileAction { DOWNLOAD, OPEN, SHARE }

fun fileActions(saved: Boolean): List<FileAction> =
    if (saved) listOf(FileAction.OPEN, FileAction.SHARE) else listOf(FileAction.DOWNLOAD)

fun humanSize(bytes: Long): String {
    val units = listOf("o", "Ko", "Mo", "Go")
    var v = bytes.toDouble()
    var u = 0
    while (v >= 1024 && u < units.size - 1) {
        v /= 1024
        u++
    }
    return if (u == 0) "$bytes o" else String.format(java.util.Locale.FRANCE, "%.1f %s", v, units[u])
}

fun sharedUris(uris: List<String>): List<String> =
    uris.filter { it.startsWith("content://", ignoreCase = true) }
