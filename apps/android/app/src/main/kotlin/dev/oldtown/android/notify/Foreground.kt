package dev.oldtown.android.notify

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
 * Notifier un message reçu ? Non si l'écran principal est au premier plan :
 * l'élément apparaît déjà dans la liste. La notification permanente du
 * service ne dépend pas de cette règle. Fonction pure, testée sans Android.
 */
fun shouldNotify(mainVisible: Boolean): Boolean = !mainVisible
