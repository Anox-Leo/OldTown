package dev.oldtown.android

import android.content.Intent
import androidx.core.content.ContextCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.oldtown.android.relay.Connection
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.android.relay.RelayService
import dev.oldtown.core.forgetDevice
import java.util.Base64
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class RelayServiceTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val store = RelayHub.store(context)

    private var ownsStore = false

    @Before
    fun noRealDevice() {
        assumeNoRealDevice(context)
        ownsStore = true
    }

    @After
    fun tearDown() {
        if (!ownsStore) return
        context.stopService(Intent(context, RelayService::class.java))
        forgetDevice(store)
    }

    @Test
    fun startsReconnectsAndStops() = runBlocking {
        store.put(
            "device.json",
            """{"device_id":"dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H5","name":"test","server_url":"ws://127.0.0.1:9"}"""
                .toByteArray(),
        )
        store.put("device.key", (Base64.getEncoder().encodeToString(ByteArray(32) { 7 }) + "\n").toByteArray())
        RelayHub.reload(context)

        ContextCompat.startForegroundService(context, Intent(context, RelayService::class.java))
        withTimeout(10_000) { RelayHub.client(context) }
        val reconnecting = withTimeout(10_000) {
            RelayHub.state.first { it.connection is Connection.Reconnecting }
        }
        assertTrue(reconnecting.connection is Connection.Reconnecting)

        context.stopService(Intent(context, RelayService::class.java))
        val stopped = withTimeout(10_000) { RelayHub.state.first { it.connection == Connection.Stopped } }
        assertEquals(Connection.Stopped, stopped.connection)
    }

    @Test
    fun unpairedServiceStopsItself() = runBlocking {
        forgetDevice(store)
        RelayHub.reload(context)
        ContextCompat.startForegroundService(context, Intent(context, RelayService::class.java))
        kotlinx.coroutines.delay(1_000)
        assertEquals(Connection.Stopped, RelayHub.state.value.connection)
    }
}
