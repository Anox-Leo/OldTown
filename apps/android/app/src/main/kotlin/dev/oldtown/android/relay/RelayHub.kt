package dev.oldtown.android.relay

import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat
import dev.oldtown.android.secrets.KeystoreSecretStore
import dev.oldtown.android.ui.ChannelTarget
import dev.oldtown.android.ui.mePerson
import dev.oldtown.android.ui.parseChannel
import dev.oldtown.android.ui.recipientsOf
import dev.oldtown.core.AdminRoot
import dev.oldtown.core.DeviceState
import dev.oldtown.core.Group
import dev.oldtown.core.OtClient
import dev.oldtown.core.OtException
import dev.oldtown.core.PersonItem
import dev.oldtown.core.Received
import dev.oldtown.core.deviceState
import dev.oldtown.core.groupSnapshot
import dev.oldtown.core.history
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout

/** État de la connexion, tel qu'affiché (notification permanente, écran). */
sealed interface Connection {
    /** Service arrêté, ou appareil pas encore appairé. */
    data object Stopped : Connection
    data object Connecting : Connection
    data class Connected(val pending: ULong) : Connection
    /** `failures` échecs de suite : au-delà de 2, on soupçonne Tailscale. */
    data class Reconnecting(val reason: String, val failures: Int) : Connection
    data class AuthFailed(val message: String) : Connection
}

/** Fait ponctuel à montrer à l'écran (bandeau), puis oublié. */
sealed interface Notice {
    data class PairingCompleted(val name: String) : Notice
    data class JoinFailed(val message: String) : Notice
    data class Problem(val message: String) : Notice
    /** Admin : une personne invitée par ce téléphone est certifiée. */
    data class PersonJoined(val name: String, val deviceName: String, val fingerprint: String) : Notice
    /** Invité : ma personne est certifiée. */
    data object Joined : Notice
}

/** Alerte de l'annuaire, gardée jusqu'au redémarrage du service (20 au plus), comme l'extension. */
sealed interface DirectoryAlert {
    /** Le relais publie pour ma personne un autre groupe que le mien. */
    data object Fork : DirectoryAlert
    data class Rejected(val userId: String, val reason: String) : DirectoryAlert
    /** Nouvelles clés (réinvitation) : à vérifier de vive voix. */
    data class KeysChanged(val userId: String) : DirectoryAlert
}

/** Fichier en cours d'envoi ou de réception (étape 5). */
data class Transfer(
    /** URI de la source (envoi) ou `msg_id` de l'élément (réception). */
    val id: String,
    val name: String,
    val upload: Boolean,
    val done: Long,
    val total: Long,
    val error: String? = null,
    /** Canal d'un envoi, pour sa bulle de progression. */
    val channel: String = "",
)

data class HubState(
    /** Appareil enregistré ; `null` avant l'appairage. */
    val device: DeviceState? = null,
    val connection: Connection = Connection.Stopped,
    /** Groupe ; `null` tant que le parrain n'a pas répondu. */
    val group: Group? = null,
    val history: List<Received> = emptyList(),
    val loaded: Boolean = false,
    val transfers: List<Transfer> = emptyList(),
    /** Personnes de l'annuaire vérifié, moi compris (étape 10) ; vide sans annuaire. */
    val people: List<PersonItem> = emptyList(),
    /** Dernière ouverture de la conversation de chaque personne (non-lus). */
    val opened: Map<String, Long> = emptyMap(),
    /** Admin sans annuaire publié : le nom qui sera signé (D51) ; `null` sinon. */
    val rootMissing: String? = null,
    val alerts: List<DirectoryAlert> = emptyList(),
    /** Invitation rachetée, en attente de la certification par l'admin. */
    val invitePending: Boolean = false,
) {
    /** Ma personne dans l'annuaire. */
    val me: PersonItem? get() = mePerson(people, device?.deviceId)

    val isAdmin: Boolean get() = me?.isAdmin == true || rootMissing != null
}

/**
 * Point d'accès des écrans à la connexion : le client lui-même appartient
 * à [RelayService], seul à le créer (note 0009). Tout est dans le même
 * processus ; les écrans observent [state] et passent par [client].
 */
object RelayHub {
    private val _state = MutableStateFlow(HubState())
    val state: StateFlow<HubState> = _state.asStateFlow()

    private val _notices = MutableSharedFlow<Notice>(extraBufferCapacity = 16)
    val notices: SharedFlow<Notice> = _notices.asSharedFlow()

    private val _client = MutableStateFlow<OtClient?>(null)

    fun store(context: Context) = KeystoreSecretStore.get(context)

    /** Relit l'état local (appareil, groupe, historique), sans connexion. */
    suspend fun reload(context: Context) = withContext(Dispatchers.IO) {
        val store = store(context)
        val device = deviceState(store)
        val group = if (device != null) groupSnapshot(store) else null
        val items = if (device != null) history(store) else emptyList()
        val openedAt = if (device != null) dev.oldtown.core.opened(store) else emptyMap()
        val pending = device != null && dev.oldtown.core.invitePending(store)
        _state.update {
            val known = device != null
            it.copy(
                device = device,
                group = group,
                history = items,
                loaded = true,
                opened = openedAt,
                invitePending = pending,
                people = if (known) it.people else emptyList(),
                rootMissing = if (known) it.rootMissing else null,
                alerts = if (known) it.alerts else emptyList(),
            )
        }
    }

    /** Démarre le service s'il y a un appareil enregistré. */
    fun ensureStarted(context: Context) {
        if (_state.value.device == null) return
        ContextCompat.startForegroundService(context, Intent(context, RelayService::class.java))
    }

    /** Redémarre le client (nouvelle URL du relais, appairage terminé…). */
    fun restart(context: Context) {
        val intent = Intent(context, RelayService::class.java).setAction(RelayService.ACTION_RESTART)
        ContextCompat.startForegroundService(context, intent)
    }

    /** Arrête le service (avant d'oublier l'appareil). */
    fun stop(context: Context) {
        context.stopService(Intent(context, RelayService::class.java))
    }

    /** Client du service, démarré au besoin. */
    suspend fun client(context: Context): OtClient {
        _client.value?.let { return it }
        if (_state.value.device == null) reload(context)
        if (_state.value.device == null) throw OtException.NotInGroup("appareil pas encore appairé")
        ensureStarted(context)
        return withTimeout(CLIENT_TIMEOUT_MS) { _client.filterNotNull().first() }
    }

    /**
     * Envoie un texte ou un lien dans un canal : un de mes appareils (`""` :
     * tous), une personne (ses appareils par défaut) ou un de ses appareils.
     */
    suspend fun send(context: Context, text: String, channel: String): String {
        val c = client(context)
        return when (val t = parseChannel(channel)) {
            is ChannelTarget.Mine -> c.sendText(text, recipientsOf(t.device))
            is ChannelTarget.Person -> c.sendTextToUser(text, t.user, t.device)
        }
    }

    /** Conversation d'une personne ouverte maintenant : ses non-lus retombent à zéro. */
    suspend fun markOpened(context: Context, user: String) = withContext(Dispatchers.IO) {
        val all = dev.oldtown.core.markOpened(store(context), user, System.currentTimeMillis())
        _state.update { it.copy(opened = all) }
    }

    /** Admin : publie l'annuaire (D51), en signant le nom donné par le relais. */
    suspend fun publishDirectory(context: Context) {
        val root = client(context).adminRoot(true)
        if (root is AdminRoot.Ready) setRootMissing(null)
        refreshPeople(context)
    }

    /** Relit les personnes de l'annuaire vérifié (sans connexion : l'état connu). */
    suspend fun refreshPeople(context: Context) {
        setPeople(client(context).people())
    }

    // --- Appelé par RelayService -------------------------------------------

    internal fun attach(client: OtClient) {
        _client.value = client
    }

    internal fun detach() {
        _client.value = null
        _state.update { it.copy(connection = Connection.Stopped) }
    }

    internal fun setConnection(connection: Connection) =
        _state.update { it.copy(connection = connection) }

    internal fun setGroup(group: Group?) = _state.update { it.copy(group = group) }

    internal fun setDevice(device: DeviceState?) = _state.update { it.copy(device = device) }

    internal fun setPeople(people: List<PersonItem>) = _state.update { it.copy(people = people) }

    internal fun setRootMissing(name: String?) = _state.update { it.copy(rootMissing = name) }

    internal fun setInvitePending(pending: Boolean) = _state.update { it.copy(invitePending = pending) }

    internal fun addAlert(alert: DirectoryAlert) = _state.update { s ->
        s.copy(alerts = (listOf(alert) + s.alerts.filter { it != alert }).take(MAX_ALERTS))
    }

    fun dismissAlerts() = _state.update { it.copy(alerts = emptyList()) }

    internal fun prepend(item: Received) = _state.update { s ->
        s.copy(history = (listOf(item) + s.history.filter { it.msgId != item.msgId }).take(HISTORY_LEN))
    }

    /** Envoi rangé (en tête) ou mis à jour sur place (accepté), note 0033. */
    internal fun upsert(item: Received) = _state.update { s ->
        if (s.history.any { it.msgId == item.msgId }) {
            s.copy(history = s.history.map { if (it.msgId == item.msgId) item else it })
        } else {
            s.copy(history = (listOf(item) + s.history).take(HISTORY_LEN))
        }
    }

    /** Envoi en échec, retiré de l'historique. */
    internal fun remove(msgId: String) = _state.update { s ->
        s.copy(history = s.history.filterNot { it.msgId == msgId })
    }

    fun setHistory(items: List<Received>) = _state.update { it.copy(history = items) }

    /** Remplace un élément de l'historique (fichier enregistré). */
    internal fun replace(item: Received) = _state.update { s ->
        s.copy(history = s.history.map { if (it.msgId == item.msgId) item else it })
    }

    internal fun setTransfer(t: Transfer) = _state.update { s ->
        s.copy(transfers = s.transfers.filterNot { it.id == t.id } + t)
    }

    internal fun removeTransfer(id: String) = _state.update { s ->
        s.copy(transfers = s.transfers.filterNot { it.id == id })
    }

    /** Envoie des fichiers partagés dans un canal, par le service (qui garde l'accès aux URI). */
    fun sendFiles(context: Context, uris: List<android.net.Uri>, channel: String) {
        if (uris.isEmpty()) return
        val clip = android.content.ClipData.newRawUri(null, uris.first())
        for (u in uris.drop(1)) clip.addItem(android.content.ClipData.Item(u))
        val intent = Intent(context, RelayService::class.java)
            .setAction(RelayService.ACTION_SEND_FILES)
            .putExtra(RelayService.EXTRA_CHANNEL, channel)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        intent.clipData = clip
        ContextCompat.startForegroundService(context, intent)
    }

    /** Télécharge le fichier d'un élément, par le service (survit à l'écran). */
    fun download(context: Context, msgId: String) {
        val intent = Intent(context, RelayService::class.java)
            .setAction(RelayService.ACTION_DOWNLOAD)
            .putExtra(RelayService.EXTRA_MSG_ID, msgId)
        ContextCompat.startForegroundService(context, intent)
    }

    internal fun notify(notice: Notice) {
        _notices.tryEmit(notice)
    }

    private const val CLIENT_TIMEOUT_MS = 10_000L
    private const val HISTORY_LEN = 100
    private const val MAX_ALERTS = 20
}
