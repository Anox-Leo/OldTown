package dev.oldtown.android.files

/**
 * Règles des fichiers reçus (étape 5, note 0018) : fonctions pures, testées
 * sans Android.
 */

/** Téléchargés d'office en dessous de cette taille, sur un réseau non facturé. */
const val AUTO_DOWNLOAD_MAX: Long = 20L * 1024 * 1024

/**
 * Télécharger d'office ? Sous 20 Mo, et seulement sur un réseau non facturé
 * à l'usage (Wi-Fi, Ethernet) : au-delà, ou en 4G, l'utilisateur choisit.
 */
fun shouldAutoDownload(size: ULong, unmetered: Boolean): Boolean =
    unmetered && size <= AUTO_DOWNLOAD_MAX.toULong()

/** Action proposée sur un fichier reçu. */
enum class FileAction { DOWNLOAD, OPEN, SHARE }

/** Actions d'un fichier reçu : le télécharger, ou, une fois enregistré, l'ouvrir et le partager. */
fun fileActions(saved: Boolean): List<FileAction> =
    if (saved) listOf(FileAction.OPEN, FileAction.SHARE) else listOf(FileAction.DOWNLOAD)

/** Taille lisible : `1,5 Mo`. */
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

/**
 * URI acceptées d'un partage : seulement `content://`. Une URI `file://`
 * pourrait désigner un fichier privé de l'appli (ses clés) qu'une autre appli
 * nous ferait envoyer.
 */
fun sharedUris(uris: List<String>): List<String> =
    uris.filter { it.startsWith("content://", ignoreCase = true) }
