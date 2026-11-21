package dev.oldtown.android

import android.app.Application
import dev.oldtown.android.notify.Notifications
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

class OldTownApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Notifications.createChannels(this)
    }

    companion object {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    }
}
