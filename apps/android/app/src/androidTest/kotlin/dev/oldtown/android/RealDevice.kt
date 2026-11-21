package dev.oldtown.android

import android.content.Context
import dev.oldtown.android.relay.RelayHub
import dev.oldtown.core.deviceState
import org.junit.Assume.assumeTrue

fun assumeNoRealDevice(context: Context) {
    assumeTrue(
        "téléphone appairé : test sauté pour ne pas effacer son identité",
        deviceState(RelayHub.store(context)) == null,
    )
}
