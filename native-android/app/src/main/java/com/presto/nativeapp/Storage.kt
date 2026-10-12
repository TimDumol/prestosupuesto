package com.presto.nativeapp

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import androidx.room.*
import com.presto.core.*
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlinx.coroutines.flow.Flow

@androidx.room.Entity(tableName = "finance_cache", primaryKeys = ["scope"])
data class CacheRow(val scope: String, val snapshot: String)

@androidx.room.Entity(tableName = "entry_drafts", primaryKeys = ["scope"])
data class DraftRow(val scope: String, val draft: String)

@androidx.room.Entity(tableName = "outbox", indices = [Index("scope")])
data class OutboxRow(
    @PrimaryKey val id: String,
    val scope: String,
    val transactionId: String,
    val payload: String,
    val optimistic: String,
    val original: String?,
    val editing: Boolean,
    val createdAt: Long = System.currentTimeMillis(),
    val status: String = "pending",
    val error: String? = null,
    val confirmedRevision: Long? = null,
)

@Dao
interface FinanceDao {
    @Query("SELECT * FROM finance_cache WHERE scope = :scope")
    suspend fun cache(scope: String): CacheRow?

    @Insert(onConflict = OnConflictStrategy.REPLACE) suspend fun putCache(row: CacheRow)

    @Query("SELECT * FROM entry_drafts WHERE scope = :scope")
    suspend fun draft(scope: String): DraftRow?

    @Insert(onConflict = OnConflictStrategy.REPLACE) suspend fun putDraft(row: DraftRow)

    @Query("SELECT * FROM outbox WHERE scope = :scope ORDER BY createdAt, rowid")
    suspend fun outbox(scope: String): List<OutboxRow>

    @Query("SELECT * FROM outbox WHERE scope = :scope ORDER BY createdAt, rowid")
    fun observeOutbox(scope: String): Flow<List<OutboxRow>>

    @Insert(onConflict = OnConflictStrategy.ABORT) suspend fun enqueue(row: OutboxRow)

    @Update suspend fun update(row: OutboxRow)

    @Query("DELETE FROM outbox WHERE id = :id") suspend fun remove(id: String)
}

@Database(
    entities = [CacheRow::class, DraftRow::class, OutboxRow::class],
    version = 1,
    exportSchema = true,
)
abstract class FinanceDatabase : RoomDatabase() {
    abstract fun finance(): FinanceDao
}

/** Keys remain in Keystore; ciphertext lives outside Android automatic backup. */
class Vault(private val context: Context, private val namespace: String = "presto") {
    private fun key(alias: String): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let {
            return it
        }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
            .apply {
                init(
                    KeyGenParameterSpec.Builder(
                            alias,
                            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
                        )
                        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                        .setKeySize(256)
                        .build()
                )
            }
            .generateKey()
    }

    fun encrypt(bytes: ByteArray, alias: String): ByteArray {
        val cipher =
            Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key(alias)) }
        return cipher.iv + cipher.doFinal(bytes)
    }

    fun decrypt(bytes: ByteArray, alias: String): ByteArray {
        require(bytes.size >= 28) { "Encrypted file is incomplete." }
        return Cipher.getInstance("AES/GCM/NoPadding")
            .apply {
                init(
                    Cipher.DECRYPT_MODE,
                    key(alias),
                    GCMParameterSpec(128, bytes.copyOfRange(0, 12)),
                )
            }
            .doFinal(bytes.copyOfRange(12, bytes.size))
    }

    private val sessionFile
        get() = File(context.noBackupFilesDir, "$namespace-pocket-session.sealed")

    fun session(): Session? =
        if (!sessionFile.exists()) null
        else
            codec.decodeFromString<Session>(
                decrypt(sessionFile.readBytes(), "$namespace-session-v1").decodeToString()
            )

    fun saveSession(session: Session) {
        atomicFile(
            sessionFile,
            encrypt(
                codec.encodeToString(Session.serializer(), session).encodeToByteArray(),
                "$namespace-session-v1",
            ),
        )
    }

    fun clearSession() {
        sessionFile.delete()
    }
}

fun atomicFile(file: File, bytes: ByteArray) {
    file.parentFile?.mkdirs()
    val temp = File(file.parentFile, file.name + ".tmp")
    java.io.FileOutputStream(temp).use {
        it.write(bytes)
        it.fd.sync()
    }
    check(temp.renameTo(file)) { "Could not publish local file." }
}
