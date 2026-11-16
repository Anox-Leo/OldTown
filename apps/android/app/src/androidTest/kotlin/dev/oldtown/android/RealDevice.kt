package dev.oldtown.android

import android.content.Context
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.core.deviceState
import org.junit.Assume.assumeTrue

/**
 * Les tests instrumentés tournent dans l'appli elle-même, avec ses vraies
 * données. Un test qui a besoin d'un appareil non appairé (ou qui en crée un
 * faux puis l'oublie) est sauté sur un téléphone appairé : il effacerait son
 * identité, et il faudrait le réappairer. Attention : un `@After` tourne
 * même quand l'hypothèse saute le test ; il ne doit rien effacer alors.
 */
fun assumeNoRealDevice(context: Context) {
    assumeTrue(
        "téléphone appairé : test sauté pour ne pas effacer son identité",
        deviceState(RelayHub.store(context)) == null,
    )
}
