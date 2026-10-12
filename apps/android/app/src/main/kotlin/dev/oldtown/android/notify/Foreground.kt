package dev.oldtown.android.notify

import android.app.ActivityManager

/**
 * Écran principal visible (étape 4, T8b) : tant qu'il l'est, un message reçu
 * s'ajoute seulement à l'historique affiché, sans notification. Tenu par
 * `MainActivity` (`onStart`/`onStop`), lu par le service.
 */
object Foreground {
    @Volatile
    var mainVisible: Boolean = false
}

/**
 * Notifier un message reçu ? Non si l'appli est au premier plan : l'élément
 * apparaît déjà dans la liste. Deux signaux, l'un ou l'autre suffit : le
 * drapeau tenu par `MainActivity`, et l'importance du processus donnée par
 * Android (`processForeground`), au cas où le premier manquerait un passage
 * au premier plan (note 0024). La notification permanente du service ne
 * dépend pas de cette règle. Fonction pure, testée sans Android.
 */
fun shouldNotify(mainVisible: Boolean, processForeground: Boolean = false): Boolean =
    !mainVisible && !processForeground

/**
 * Un écran de l'appli est-il visible ? Android classe alors le processus en
 * `IMPORTANCE_FOREGROUND` ; le service de premier plan seul ne donne que
 * `IMPORTANCE_FOREGROUND_SERVICE`.
 */
fun processInForeground(): Boolean {
    val info = ActivityManager.RunningAppProcessInfo()
    ActivityManager.getMyMemoryState(info)
    return info.importance == ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
}

/** Notifications de messages affichées au plus en même temps. */
const val MAX_SHOWN_MESSAGES = 3

/**
 * Identifiants à retirer pour n'en garder que `max` : les plus anciennes
 * d'abord (`posted` : identifiant et date de publication). Fonction pure,
 * testée sans Android.
 */
fun oldestBeyond(posted: List<Pair<Int, Long>>, max: Int): List<Int> =
    posted.sortedBy { it.second }.dropLast(max).map { it.first }
