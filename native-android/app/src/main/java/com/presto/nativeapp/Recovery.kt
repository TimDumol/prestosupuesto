package com.presto.nativeapp

import android.content.Context
import android.util.JsonReader
import com.presto.core.*
import java.io.*
import java.security.DigestInputStream
import java.security.MessageDigest
import java.time.Instant
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.*

@Serializable
data class RecoveryInfo(
    val id: String,
    val createdAt: String,
    val sha256: String,
    val byteLength: Int,
    val databaseSchemaVersion: Int,
    val exportedAt: String,
    val operationWatermark: String?,
    val tableCounts: JsonObject,
)

data class VerifiedRecovery(val info: RecoveryInfo, val raw: String)

fun sha256(bytes: ByteArray) = hex(MessageDigest.getInstance("SHA-256").digest(bytes))

private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }

private data class InterchangeMetadata(
    val schema: Int,
    val exportedAt: String,
    val watermark: String?,
    val counts: JsonObject,
)

/** Streaming validation and chunk encryption keep large recovery archives off the app heap. */
class RecoveryStore(private val context: Context, private val vault: Vault) {
    private val prefs = context.getSharedPreferences("recovery-attempts", Context.MODE_PRIVATE)

    private fun directory(scope: String) =
        File(context.noBackupFilesDir, "finance-recovery/${sha256(scope.encodeToByteArray())}")
            .apply { mkdirs() }

    private fun alias(scope: String) = "presto-recovery-${sha256(scope.encodeToByteArray())}"

    fun reserve(scope: String, now: Long = System.currentTimeMillis()): Boolean {
        val lastAttempt = prefs.getLong("$scope-attempt", 0)
        val lastSuccess = prefs.getLong("$scope-success", 0)
        if (
            (lastAttempt > 0 && now - lastAttempt < 15 * 60 * 1000) ||
                (lastSuccess > 0 && now - lastSuccess < 6 * 60 * 60 * 1000)
        )
            return false
        check(prefs.edit().putLong("$scope-attempt", now).commit()) {
            "Could not persist backup attempt."
        }
        return true
    }

    private fun validate(stream: InputStream): InterchangeMetadata {
        var format: Int? = null
        var schema: Int? = null
        var exported: String? = null
        var watermark: String? = null
        var sawWatermark = false
        val counts = linkedMapOf<String, Int>()
        val actual = linkedMapOf<String, Int>()
        val fields = mutableSetOf<String>()
        JsonReader(InputStreamReader(stream, Charsets.UTF_8)).use { reader ->
            reader.beginObject()
            while (reader.hasNext()) {
                val field = reader.nextName()
                require(fields.add(field)) { "Duplicate recovery metadata." }
                when (field) {
                    "interchangeFormatVersion" -> format = reader.nextInt()
                    "databaseSchemaVersion" -> schema = reader.nextInt()
                    "exportedAt" -> exported = reader.nextString()
                    "operationWatermark" -> {
                        sawWatermark = true
                        if (reader.peek() == android.util.JsonToken.NULL) reader.nextNull()
                        else watermark = reader.nextString()
                    }
                    "tableCounts" -> {
                        reader.beginObject()
                        while (reader.hasNext()) {
                            val name = reader.nextName()
                            require(name !in counts)
                            val count = reader.nextInt()
                            require(count >= 0)
                            counts[name] = count
                        }
                        reader.endObject()
                    }
                    "tables" -> {
                        reader.beginObject()
                        while (reader.hasNext()) {
                            val name = reader.nextName()
                            require(name !in actual)
                            var count = 0
                            reader.beginArray()
                            while (reader.hasNext()) {
                                require(reader.peek() == android.util.JsonToken.BEGIN_OBJECT)
                                reader.skipValue()
                                count++
                            }
                            reader.endArray()
                            actual[name] = count
                        }
                        reader.endObject()
                    }
                    else -> reader.skipValue()
                }
            }
            reader.endObject()
            require(reader.peek() == android.util.JsonToken.END_DOCUMENT)
        }
        require(format == 1 && schema != null && schema!! in 1..16) {
            "Unsupported recovery format or schema."
        }
        require(!exported.isNullOrBlank() && sawWatermark) { "Missing recovery metadata." }
        val required =
            setOf(
                "finance_accounts",
                "finance_account_opening_balances",
                "finance_categories",
                "category_budget_periods",
                "finance_transactions",
                "finance_transaction_revisions",
                "transaction_valuations",
                "transaction_valuation_source_cells",
                "budget_month_snapshots",
                "budget_month_closures",
                "budget_opening_state",
                "finance_exchange_rate_source_rows",
                "exchange_rates",
                "finance_account_actuals",
                "finance_account_balance_checkpoints",
                "finance_operations",
                "finance_category_corrections",
                "finance_category_correction_items",
                "reconciliation_batches",
                "reconciliation_items",
                "finance_import_runs",
                "finance_quarantined_rows",
                "finance_source_artifacts",
            ) +
                (if (schema!! >= 11) setOf("budget_income_expectations") else emptySet()) +
                (if (schema!! >= 14)
                    setOf(
                        "budget_ledger_state",
                        "budget_plan_income",
                        "budget_income_receipts",
                        "budget_ledger_movements",
                        "budget_coverage_closures",
                    )
                else emptySet())
        require(counts.keys == required && actual == counts) {
            "Incomplete finance table set or counts."
        }
        return InterchangeMetadata(
            schema!!,
            exported!!,
            watermark,
            buildJsonObject { counts.forEach { (table, count) -> put(table, count) } },
        )
    }

    fun save(scope: String, raw: String, digest: String): RecoveryInfo {
        val temporary = File.createTempFile("recovery-input-", ".json", context.cacheDir)
        return try {
            temporary.writeText(raw)
            saveFile(scope, temporary, digest)
        } finally {
            temporary.delete()
        }
    }

    fun saveFile(scope: String, source: File, digest: String): RecoveryInfo {
        require(digest.matches(Regex("[0-9a-fA-F]{64}"))) { "Missing recovery digest." }
        require(source.length() <= Int.MAX_VALUE) { "Recovery snapshot is too large." }
        val hash = MessageDigest.getInstance("SHA-256")
        val metadata = DigestInputStream(source.inputStream(), hash).use(::validate)
        require(hex(hash.digest()).equals(digest, true)) { "Recovery digest mismatch." }
        val id = uuidV7()
        val root = directory(scope)
        val generation = File(root, "$id.partial").apply { mkdirs() }
        try {
            source.inputStream().use { input ->
                val buffer = ByteArray(64 * 1024)
                var index = 0
                while (true) {
                    val length = input.read(buffer)
                    if (length < 0) break
                    val chunk = buffer.copyOf(length)
                    val encrypted = vault.encrypt(chunk, alias(scope))
                    val file = File(generation, "${index++}.sealed")
                    atomicFile(file, encrypted)
                    check(vault.decrypt(file.readBytes(), alias(scope)).contentEquals(chunk)) {
                        "Recovery encryption verification failed."
                    }
                }
            }
            val info =
                RecoveryInfo(
                    id,
                    Instant.now().toString(),
                    digest.lowercase(),
                    source.length().toInt(),
                    metadata.schema,
                    metadata.exportedAt,
                    metadata.watermark,
                    metadata.counts,
                )
            atomicFile(
                File(generation, "metadata.json"),
                codec.encodeToString(RecoveryInfo.serializer(), info).encodeToByteArray(),
            )
            verify(scope, generation)
            check(generation.renameTo(File(root, id))) { "Could not publish recovery generation." }
            root
                .listFiles()
                ?.filter { it.isDirectory && !it.name.endsWith(".partial") }
                ?.sortedByDescending { it.name }
                ?.drop(8)
                ?.forEach { it.deleteRecursively() }
            check(prefs.edit().putLong("$scope-success", System.currentTimeMillis()).commit())
            return info
        } catch (e: Exception) {
            generation.deleteRecursively()
            throw e
        }
    }

    private fun decryptedStream(scope: String, dir: File): InputStream {
        val chunks =
            dir.listFiles()
                ?.filter { it.extension == "sealed" }
                ?.sortedBy { it.nameWithoutExtension.toInt() } ?: error("Recovery is missing.")
        require(
            chunks.isNotEmpty() &&
                chunks.map { it.nameWithoutExtension.toInt() } == chunks.indices.toList()
        )
        return object : InputStream() {
            var index = 0
            var current: ByteArrayInputStream? = null

            override fun read(): Int {
                val one = ByteArray(1)
                return if (read(one, 0, 1) < 0) -1 else one[0].toInt() and 255
            }

            override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
                if (length == 0) return 0
                while (true) {
                    val available = current?.read(buffer, offset, length) ?: -1
                    if (available >= 0) return available
                    if (index >= chunks.size) return -1
                    current =
                        ByteArrayInputStream(
                            vault.decrypt(chunks[index++].readBytes(), alias(scope))
                        )
                }
            }

            override fun close() {
                current?.close()
                current = null
                index = chunks.size
            }
        }
    }

    private fun verify(scope: String, dir: File): RecoveryInfo {
        val info = codec.decodeFromString<RecoveryInfo>(File(dir, "metadata.json").readText())
        val hash = MessageDigest.getInstance("SHA-256")
        var byteLength = 0
        val counting =
            object : FilterInputStream(decryptedStream(scope, dir)) {
                override fun read(buffer: ByteArray, offset: Int, length: Int): Int =
                    super.read(buffer, offset, length).also { if (it > 0) byteLength += it }

                override fun read(): Int = super.read().also { if (it >= 0) byteLength++ }
            }
        val metadata = DigestInputStream(counting, hash).use(::validate)
        require(hex(hash.digest()) == info.sha256 && byteLength == info.byteLength) {
            "Recovery digest mismatch."
        }
        require(
            metadata.schema == info.databaseSchemaVersion &&
                metadata.exportedAt == info.exportedAt &&
                metadata.watermark == info.operationWatermark &&
                metadata.counts == info.tableCounts
        ) {
            "Recovery metadata mismatch."
        }
        return info
    }

    private fun generations(scope: String) =
        directory(scope)
            .listFiles()
            ?.filter { it.isDirectory && !it.name.endsWith(".partial") }
            ?.sortedByDescending { it.name }
            .orEmpty()

    fun latestInfo(scope: String): RecoveryInfo? =
        generations(scope).firstNotNullOfOrNull { runCatching { verify(scope, it) }.getOrNull() }

    fun latest(scope: String): VerifiedRecovery? =
        latestInfo(scope)?.let { info ->
            VerifiedRecovery(
                info,
                decryptedStream(scope, File(directory(scope), info.id)).bufferedReader().use {
                    it.readText()
                },
            )
        }

    fun materialize(scope: String, info: RecoveryInfo): File {
        verify(scope, File(directory(scope), info.id))
        val file = File.createTempFile("recovery-restore-", ".json", context.cacheDir)
        return try {
            decryptedStream(scope, File(directory(scope), info.id)).use { input ->
                FileOutputStream(file).use { output ->
                    input.copyTo(output)
                    output.fd.sync()
                }
            }
            file
        } catch (e: Exception) {
            file.delete()
            throw e
        }
    }
}
