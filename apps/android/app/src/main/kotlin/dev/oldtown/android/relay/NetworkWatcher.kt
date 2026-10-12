package dev.oldtown.android.relay

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network

/**
 * Passage Wi-Fi ↔ 4G : quand le réseau par défaut change, la connexion
 * en cours est peut-être morte ; on se reconnecte sans attendre le délai de
 * reconnexion (critère « moins de 10 s »).
 */
class NetworkWatcher(context: Context, private val onChange: () -> Unit) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private var current: Network? = null

    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            // Le premier appel décrit le réseau déjà en place : rien à faire.
            val previous = current
            current = network
            if (previous != null && previous != network) onChange()
        }
    }

    fun start() = connectivity.registerDefaultNetworkCallback(callback)

    fun stop() = connectivity.unregisterNetworkCallback(callback)
}
