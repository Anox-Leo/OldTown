package dev.oldtown.android.relay

import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat
import dev.oldtown.android.secrets.KeystoreSecretStore
import dev.oldtown.core.DeviceState
import dev.oldtown.core.Group
import dev.oldtown.core.OtClient
import dev.oldtown.core.OtException
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
}

data class HubState(
    /** Appareil enregistré ; `null` avant l'appairage. */
    val device: DeviceState? = null,
    val connection: Connection = Connection.Stopped,
    /** Groupe ; `null` tant que le parrain n'a pas répondu. */
    val group: Group? = null,
    val history: List<Received> = emptyList(),
    val loaded: Boolean = false,
)

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
        _state.update { it.copy(device = device, group = group, history = items, loaded = true) }
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

    /** Envoie un texte ou un lien ; `to` vide : tous les autres appareils. */
    suspend fun send(context: Context, text: String, to: List<String>): String =
        client(context).sendText(text, to)

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

    internal fun prepend(item: Received) = _state.update { s ->
        s.copy(history = (listOf(item) + s.history.filter { it.msgId != item.msgId }).take(HISTORY_LEN))
    }

    fun setHistory(items: List<Received>) = _state.update { it.copy(history = items) }

    internal fun notify(notice: Notice) {
        _notices.tryEmit(notice)
    }

    private const val CLIENT_TIMEOUT_MS = 10_000L
    private const val HISTORY_LEN = 50
}
