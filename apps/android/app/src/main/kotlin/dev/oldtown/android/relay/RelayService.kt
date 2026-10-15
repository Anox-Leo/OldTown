package dev.oldtown.android.relay

import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.util.Log
import androidx.core.app.ServiceCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import android.net.Uri
import dev.oldtown.android.OldTownApp
import dev.oldtown.android.files.FileTransfers
import dev.oldtown.android.files.shouldAutoDownload
import dev.oldtown.android.notify.Foreground
import dev.oldtown.android.notify.Notifications
import dev.oldtown.android.notify.processInForeground
import dev.oldtown.android.notify.shouldNotify
import dev.oldtown.android.share.Shortcuts
import dev.oldtown.core.Event
import dev.oldtown.core.OtClient
import dev.oldtown.core.OtException
import dev.oldtown.core.Received
import dev.oldtown.core.deviceState
import dev.oldtown.core.history
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Service de premier plan `specialUse` (note 0016) : tient l'unique
 * connexion de l'appareil au relais, de l'ouverture de l'appli ou du
 * démarrage du téléphone jusqu'à l'oubli de l'appareil. Fait aussi les
 * transferts de fichiers (note 0027) : il garde l'accès aux URI partagées et
 * survit à la fermeture des écrans.
 */
class RelayService : LifecycleService() {
    private var client: OtClient? = null
    private var loop: Job? = null
    private var network: NetworkWatcher? = null

    override fun onCreate() {
        super.onCreate()
        ServiceCompat.startForeground(
            this,
            Notifications.CONNECTION_ID,
            Notifications.connection(this, Connection.Connecting, waitingSponsor = false),
            // `specialUse` existe depuis Android 14 ; Android 13 (le téléphone
            // visé par minSdk 33) n'exige pas de type.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
            } else {
                0
            },
        )
        network = NetworkWatcher(this) { client?.reconnectNow() }.also { it.start() }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        if (intent?.action == ACTION_RESTART) stopClient()
        if (loop == null) loop = lifecycleScope.launch(Dispatchers.IO) { run() }
        when (intent?.action) {
            ACTION_SEND_FILES -> {
                val clip = intent.clipData
                val uris = (0 until (clip?.itemCount ?: 0)).mapNotNull { clip?.getItemAt(it)?.uri }
                val to = intent.getStringArrayListExtra(EXTRA_TO).orEmpty()
                lifecycleScope.launch(Dispatchers.IO) { sendFiles(uris, to) }
            }
            ACTION_DOWNLOAD -> intent.getStringExtra(EXTRA_MSG_ID)?.let { msgId ->
                lifecycleScope.launch(Dispatchers.IO) { download(msgId, userAsked = true) }
            }
        }
        return START_STICKY
    }

    private suspend fun sendFiles(uris: List<Uri>, to: List<String>) {
        val c = try {
            RelayHub.client(this)
        } catch (e: OtException) {
            RelayHub.notify(Notice.Problem(e.message ?: "envoi impossible"))
            return
        }
        for (uri in uris) FileTransfers.send(this, c, uri, to)
    }

    /**
     * Télécharge le fichier d'un élément ; notifie l'enregistrement si
     * l'appli n'est pas au premier plan, ou si l'utilisateur l'a demandé
     * depuis une notification.
     */
    private suspend fun download(msgId: String, userAsked: Boolean) {
        val c = try {
            RelayHub.client(this)
        } catch (e: OtException) {
            RelayHub.notify(Notice.Problem(e.message ?: "téléchargement impossible"))
            return
        }
        val item = RelayHub.state.value.history.firstOrNull { it.msgId == msgId }
            ?: history(RelayHub.store(this)).firstOrNull { it.msgId == msgId }
            ?: return
        val saved = FileTransfers.download(this, c, item) ?: return
        if (userAsked || shouldNotify(Foreground.mainVisible, processInForeground())) {
            Notifications.showMessage(this, saved)
        }
    }

    /** Fichier annoncé : téléchargé d'office (petit, réseau non facturé), sinon proposé. */
    private fun onFile(item: Received) {
        val file = item.file ?: return
        if (shouldAutoDownload(file.size, FileTransfers.isUnmetered(this))) {
            lifecycleScope.launch(Dispatchers.IO) { download(item.msgId, userAsked = false) }
        } else if (shouldNotify(Foreground.mainVisible, processInForeground())) {
            Notifications.showMessage(this, item)
        }
    }

    private suspend fun run() {
        val store = RelayHub.store(this)
        val started = try {
            if (deviceState(store) == null) null else OtClient.start(store)
        } catch (e: OtException) {
            Log.w(TAG, "client non démarré : ${e.message}")
            RelayHub.notify(Notice.Problem(e.message ?: "client non démarré"))
            null
        }
        if (started == null) {
            stopSelf()
            return
        }
        client = started
        RelayHub.setDevice(started.device())
        RelayHub.setGroup(started.group())
        RelayHub.setConnection(Connection.Connecting)
        RelayHub.attach(started)
        Shortcuts.update(this, RelayHub.state.value.group, RelayHub.state.value.history)
        render()

        var failures = 0
        while (true) {
            val event = started.nextEvent() ?: break
            when (event) {
                is Event.Connected -> {
                    failures = 0
                    RelayHub.setConnection(Connection.Connected(event.pending))
                }
                is Event.Disconnected -> {
                    failures += 1
                    RelayHub.setConnection(Connection.Reconnecting(event.reason, failures))
                }
                is Event.AuthFailed -> RelayHub.setConnection(Connection.AuthFailed(event.message))
                is Event.Message -> {
                    RelayHub.prepend(event.received)
                    if (event.received.file != null) {
                        onFile(event.received)
                    } else if (shouldNotify(Foreground.mainVisible, processInForeground())) {
                        Notifications.showMessage(this, event.received)
                    }
                }
                // Envoi rangé, accepté ou en échec (note 0033) : jamais notifié.
                is Event.Sent -> RelayHub.upsert(event.item)
                is Event.Unsent -> RelayHub.remove(event.msgId)
                is Event.GroupChanged -> {
                    val group = started.group()
                    RelayHub.setGroup(group)
                    Shortcuts.update(this, group, RelayHub.state.value.history)
                }
                is Event.PairingCompleted -> RelayHub.notify(Notice.PairingCompleted(event.name))
                is Event.JoinFailed -> RelayHub.notify(Notice.JoinFailed(event.message))
                is Event.Rejected -> Log.w(TAG, "message de ${event.fromName} rejeté : ${event.reason}")
                is Event.StorageError -> RelayHub.notify(Notice.Problem(event.message))
            }
            render()
        }
    }

    /** Met à jour la notification permanente. */
    private fun render() {
        val state = RelayHub.state.value
        Notifications.updateConnection(
            this,
            state.connection,
            waitingSponsor = state.device != null && state.group == null,
        )
    }

    private fun stopClient() {
        loop?.cancel()
        loop = null
        val c = client ?: return
        client = null
        RelayHub.detach()
        // Fermeture propre (acquittements envoyés) sans bloquer le thread principal.
        OldTownApp.scope.launch {
            withTimeoutOrNull(SHUTDOWN_TIMEOUT_MS) { c.shutdown() }
            c.close()
        }
    }

    override fun onDestroy() {
        network?.stop()
        network = null
        stopClient()
        super.onDestroy()
    }

    companion object {
        const val ACTION_RESTART = "dev.oldtown.android.action.RESTART"
        const val ACTION_SEND_FILES = "dev.oldtown.android.action.SEND_FILES"
        const val ACTION_DOWNLOAD = "dev.oldtown.android.action.DOWNLOAD"
        const val EXTRA_TO = "to"
        const val EXTRA_MSG_ID = "msg_id"
        private const val TAG = "OldTown"
        private const val SHUTDOWN_TIMEOUT_MS = 3_000L
    }
}
