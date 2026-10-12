package dev.oldtown.android.secrets

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import dev.oldtown.core.SecretStore
import dev.oldtown.core.StoreException
import java.io.File
import java.io.IOException
import java.security.GeneralSecurityException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Stockage des secrets du cœur Rust (étape 3, T4) : chaque valeur est
 * chiffrée en AES-256-GCM par une clé non exportable du Keystore Android,
 * puis rangée dans `noBackupFilesDir/secrets/<nom>.bin` (hors sauvegardes).
 *
 * Format d'un fichier : `version (1) ‖ IV (12) ‖ chiffré ‖ tag (16)` ; le nom
 * est la donnée associée, si bien qu'un fichier renommé ne se déchiffre pas.
 * Le Keystore ne garde pas les clés Ed25519/X25519 elles-mêmes : le cœur doit
 * pouvoir s'en servir.
 */
class KeystoreSecretStore(
    private val dir: File,
    private val alias: String = DEFAULT_ALIAS,
) : SecretStore {

    private val lock = Any()

    override fun get(name: String): ByteArray? = synchronized(lock) {
        val file = fileOf(name)
        if (!file.exists()) return null
        val data = try {
            AtomicFile(file).readFully()
        } catch (e: IOException) {
            throw StoreException.Failed("lecture de $name : ${e.message}")
        }
        try {
            if (data.size < 1 + IV_LEN + TAG_LEN || data[0] != VERSION) {
                throw StoreException.Failed("$name : format inconnu")
            }
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(TAG_LEN * 8, data, 1, IV_LEN))
            cipher.updateAAD(name.toByteArray(Charsets.UTF_8))
            cipher.doFinal(data, 1 + IV_LEN, data.size - 1 - IV_LEN)
        } catch (e: GeneralSecurityException) {
            // Clé du Keystore perdue (restauration, réinitialisation) ou fichier altéré.
            throw StoreException.Failed("$name ne se déchiffre pas : ${e.javaClass.simpleName}")
        }
    }

    override fun put(name: String, value: ByteArray) {
        synchronized(lock) {
            val file = fileOf(name)
            try {
                val cipher = Cipher.getInstance(TRANSFORMATION)
                cipher.init(Cipher.ENCRYPT_MODE, key())
                cipher.updateAAD(name.toByteArray(Charsets.UTF_8))
                val sealed = cipher.doFinal(value)
                val iv = cipher.iv
                check(iv.size == IV_LEN) { "IV inattendu" }
                dir.mkdirs()
                val atomic = AtomicFile(file)
                val out = atomic.startWrite()
                try {
                    out.write(byteArrayOf(VERSION))
                    out.write(iv)
                    out.write(sealed)
                    atomic.finishWrite(out)
                } catch (e: IOException) {
                    atomic.failWrite(out)
                    throw e
                }
            } catch (e: GeneralSecurityException) {
                throw StoreException.Failed("chiffrement de $name : ${e.javaClass.simpleName}")
            } catch (e: IOException) {
                throw StoreException.Failed("écriture de $name : ${e.message}")
            } finally {
                // Le clair ne traîne pas en mémoire plus que nécessaire.
                value.fill(0)
            }
        }
    }

    override fun delete(name: String) {
        synchronized(lock) {
            val file = fileOf(name)
            AtomicFile(file).delete()
            if (file.exists()) throw StoreException.Failed("suppression de $name impossible")
        }
    }

    private fun fileOf(name: String): File {
        if (!NAME.matches(name)) throw StoreException.Failed("nom de secret invalide : $name")
        return File(dir, "$name.bin")
    }

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (ks.getKey(alias, null) as? SecretKey)?.let { return it }
        val spec = KeyGenParameterSpec.Builder(
            alias,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setRandomizedEncryptionRequired(true)
            // Utilisable téléphone verrouillé : le service reçoit la nuit.
            .setUserAuthenticationRequired(false)
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE).run {
            init(spec)
            generateKey()
        }
    }

    /** Efface la clé du Keystore (tests). */
    fun deleteKey() {
        synchronized(lock) {
            KeyStore.getInstance(KEYSTORE).apply { load(null) }.deleteEntry(alias)
        }
    }

    companion object {
        const val DEFAULT_ALIAS = "oldtown-secrets"
        private const val KEYSTORE = "AndroidKeyStore"
        private const val TRANSFORMATION = "AES/GCM/NoPadding"
        private const val VERSION: Byte = 1
        private const val IV_LEN = 12
        private const val TAG_LEN = 16
        private val NAME = Regex("[a-z0-9][a-z0-9._-]{0,63}")

        @Volatile private var instance: KeystoreSecretStore? = null

        /** Stockage de l'appli, partagé par tout le processus. */
        fun get(context: Context): KeystoreSecretStore =
            instance ?: synchronized(this) {
                instance ?: KeystoreSecretStore(File(context.noBackupFilesDir, "secrets"))
                    .also { instance = it }
            }
    }
}
