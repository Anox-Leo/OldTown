package dev.oldtown.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.oldtown.android.secrets.KeystoreSecretStore
import dev.oldtown.core.StoreException
import java.io.File
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/** Étape 3, T4 : secrets chiffrés par une clé du Keystore. */
@RunWith(AndroidJUnit4::class)
class KeystoreSecretStoreTest {
    private lateinit var dir: File
    private lateinit var store: KeystoreSecretStore

    @Before
    fun setUp() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        dir = File(context.noBackupFilesDir, "secrets-test").apply { deleteRecursively() }
        store = KeystoreSecretStore(dir, alias = "oldtown-test")
    }

    @After
    fun tearDown() {
        store.deleteKey()
        dir.deleteRecursively()
    }

    @Test
    fun roundtripAndDelete() {
        val secret = "graine-ed25519".toByteArray()
        store.put("device.key", secret.copyOf())
        assertArrayEquals(secret, store.get("device.key"))
        // Rien en clair sur le disque.
        val raw = File(dir, "device.key.bin").readBytes()
        assertFalse(String(raw, Charsets.ISO_8859_1).contains("graine"))

        store.delete("device.key")
        assertNull(store.get("device.key"))
        store.delete("device.key") // sans effet
        assertNull(store.get("absent.json"))
    }

    @Test
    fun renamedOrTamperedFileIsRefused() {
        store.put("group.json", "{}".toByteArray())
        val file = File(dir, "group.json.bin")
        file.copyTo(File(dir, "roster.json.bin"))
        assertThrows(StoreException::class.java) { store.get("roster.json") }

        val bytes = file.readBytes()
        bytes[bytes.size - 1] = (bytes[bytes.size - 1].toInt() xor 1).toByte()
        file.writeBytes(bytes)
        assertThrows(StoreException::class.java) { store.get("group.json") }
    }

    @Test
    fun lostKeystoreKeyIsReported() {
        store.put("group.json", "{}".toByteArray())
        store.deleteKey()
        assertThrows(StoreException::class.java) { store.get("group.json") }
    }

    @Test
    fun pathLikeNamesAreRefused() {
        for (name in listOf("../x", "a/b", ".cache", "", "Majuscule")) {
            assertThrows(name, StoreException::class.java) { store.put(name, byteArrayOf(1)) }
        }
    }
}
