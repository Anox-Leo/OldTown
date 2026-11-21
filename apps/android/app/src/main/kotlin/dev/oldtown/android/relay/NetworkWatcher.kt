package dev.oldtown.android.relay

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network

class NetworkWatcher(context: Context, private val onChange: () -> Unit) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private var current: Network? = null

    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            val previous = current
            current = network
            if (previous != null && previous != network) onChange()
        }
    }

    fun start() = connectivity.registerDefaultNetworkCallback(callback)

    fun stop() = connectivity.unregisterNetworkCallback(callback)
}
