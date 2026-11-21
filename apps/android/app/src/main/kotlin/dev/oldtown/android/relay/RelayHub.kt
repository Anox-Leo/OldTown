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

sealed interface Connection {
    data object Stopped : Connection
    data object Connecting : Connection
    data class Connected(val pending: ULong) : Connection
    data class Reconnecting(val reason: String, val failures: Int) : Connection
    data class AuthFailed(val message: String) : Connection
}

sealed interface Notice {
    data class PairingCompleted(val name: String) : Notice
    data class JoinFailed(val message: String) : Notice
    data class Problem(val message: String) : Notice
    data class PersonJoined(val name: String, val deviceName: String, val fingerprint: String) : Notice
    data object Joined : Notice
}

sealed interface DirectoryAlert {
    data object Fork : DirectoryAlert
    data class Rejected(val userId: String, val reason: String) : DirectoryAlert
    data class KeysChanged(val userId: String) : DirectoryAlert
}

data class Transfer(
    val id: String,
    val name: String,
    val upload: Boolean,
    val done: Long,
    val total: Long,
    val error: String? = null,
    val channel: String = "",
)

data class HubState(
    val device: DeviceState? = null,
    val connection: Connection = Connection.Stopped,
    val group: Group? = null,
    val history: List<Received> = emptyList(),
    val loaded: Boolean = false,
    val transfers: List<Transfer> = emptyList(),
    val people: List<PersonItem> = emptyList(),
    val opened: Map<String, Long> = emptyMap(),
    val rootMissing: String? = null,
    val alerts: List<DirectoryAlert> = emptyList(),
    val invitePending: Boolean = false,
) {
    val me: PersonItem? get() = mePerson(people, device?.deviceId)

    val isAdmin: Boolean get() = me?.isAdmin == true || rootMissing != null
}

object RelayHub {
    private val _state = MutableStateFlow(HubState())
    val state: StateFlow<HubState> = _state.asStateFlow()

    private val _notices = MutableSharedFlow<Notice>(extraBufferCapacity = 16)
    val notices: SharedFlow<Notice> = _notices.asSharedFlow()

    private val _client = MutableStateFlow<OtClient?>(null)

    fun store(context: Context) = KeystoreSecretStore.get(context)

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

    fun ensureStarted(context: Context) {
        if (_state.value.device == null) return
        ContextCompat.startForegroundService(context, Intent(context, RelayService::class.java))
    }

    fun restart(context: Context) {
        val intent = Intent(context, RelayService::class.java).setAction(RelayService.ACTION_RESTART)
        ContextCompat.startForegroundService(context, intent)
    }

    fun stop(context: Context) {
        context.stopService(Intent(context, RelayService::class.java))
    }

    suspend fun client(context: Context): OtClient {
        _client.value?.let { return it }
        if (_state.value.device == null) reload(context)
        if (_state.value.device == null) throw OtException.NotInGroup("appareil pas encore appairé")
        ensureStarted(context)
        return withTimeout(CLIENT_TIMEOUT_MS) { _client.filterNotNull().first() }
    }

    suspend fun send(context: Context, text: String, channel: String): String {
        val c = client(context)
        return when (val t = parseChannel(channel)) {
            is ChannelTarget.Mine -> c.sendText(text, recipientsOf(t.device))
            is ChannelTarget.Person -> c.sendTextToUser(text, t.user, t.device)
        }
    }

    suspend fun markOpened(context: Context, user: String) = withContext(Dispatchers.IO) {
        val all = dev.oldtown.core.markOpened(store(context), user, System.currentTimeMillis())
        _state.update { it.copy(opened = all) }
    }

    suspend fun publishDirectory(context: Context) {
        val root = client(context).adminRoot(true)
        if (root is AdminRoot.Ready) setRootMissing(null)
        refreshPeople(context)
    }

    suspend fun refreshPeople(context: Context) {
        setPeople(client(context).people())
    }

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

    internal fun upsert(item: Received) = _state.update { s ->
        if (s.history.any { it.msgId == item.msgId }) {
            s.copy(history = s.history.map { if (it.msgId == item.msgId) item else it })
        } else {
            s.copy(history = (listOf(item) + s.history).take(HISTORY_LEN))
        }
    }

    internal fun remove(msgId: String) = _state.update { s ->
        s.copy(history = s.history.filterNot { it.msgId == msgId })
    }

    fun setHistory(items: List<Received>) = _state.update { it.copy(history = items) }

    internal fun replace(item: Received) = _state.update { s ->
        s.copy(history = s.history.map { if (it.msgId == item.msgId) item else it })
    }

    internal fun setTransfer(t: Transfer) = _state.update { s ->
        s.copy(transfers = s.transfers.filterNot { it.id == t.id } + t)
    }

    internal fun removeTransfer(id: String) = _state.update { s ->
        s.copy(transfers = s.transfers.filterNot { it.id == id })
    }

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
